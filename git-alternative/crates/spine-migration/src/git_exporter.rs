use crate::git_format::{
    build_pack_and_index, serialize_git_commit, serialize_git_tag, serialize_git_tree,
    serialize_loose_object, write_git_refs, GitAnnotatedTag, GitCommit, GitHead, GitObjectType,
    GitOid, GitRefMap, GitTreeEntry, PackObjectToBuild,
};
use spine_core::{FileMode, HashAlgorithm, ObjectId, ObjectType};
use spine_format::{deserialize_annotated_tag, deserialize_commit, deserialize_tree};
use spine_object_store::{CompositeObjectStore, FileSystemObjectStore, ObjectStore};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GitExportError {
    #[error("I/O error during Git export: {0}")]
    Io(#[from] io::Error),
    #[error("Spine object store error: {0}")]
    ObjectStore(#[from] spine_object_store::ObjectStoreError),
    #[error("Spine format error: {0}")]
    Format(#[from] spine_format::FormatError),
    #[error("Spine history error: {0}")]
    History(#[from] spine_history::HistoryError),
    #[error("Pack builder error: {0}")]
    Pack(#[from] crate::git_format::PackBuildError),
    #[error("Ref error: {0}")]
    Ref(#[from] crate::git_format::RefError),
    #[error("Spine repository not found at {0}")]
    RepoNotFound(PathBuf),
}

#[derive(Debug, Clone, Default)]
pub struct ExportStats {
    pub blobs_count: usize,
    pub trees_count: usize,
    pub commits_count: usize,
    pub tags_count: usize,
    pub branches_count: usize,
    pub packfile_bytes: usize,
}

pub struct GitExporter {
    spine_dir: PathBuf,
    target_git_dir: PathBuf,
    is_bare: bool,
    worktree_dir: Option<PathBuf>,
    spine_to_git: HashMap<ObjectId, GitOid>,
    git_to_spine: HashMap<GitOid, ObjectId>,
}

impl GitExporter {
    pub fn new<S: AsRef<Path>, T: AsRef<Path>>(
        spine_repo_root: S,
        target_path: T,
        is_bare: bool,
    ) -> Result<Self, GitExportError> {
        let spine_root = spine_repo_root.as_ref();
        let spine_dir = if spine_root.join(".spine").exists() {
            spine_root.join(".spine")
        } else if spine_root.join("objects").exists() && spine_root.join("refs").exists() {
            spine_root.to_path_buf()
        } else {
            return Err(GitExportError::RepoNotFound(spine_root.to_path_buf()));
        };

        let target = target_path.as_ref();
        let (target_git_dir, worktree_dir) = if is_bare {
            (target.to_path_buf(), None)
        } else {
            (target.join(".git"), Some(target.to_path_buf()))
        };

        Ok(Self {
            spine_dir,
            target_git_dir,
            is_bare,
            worktree_dir,
            spine_to_git: HashMap::new(),
            git_to_spine: HashMap::new(),
        })
    }

    pub fn spine_to_git_map(&self) -> &HashMap<ObjectId, GitOid> {
        &self.spine_to_git
    }

    pub fn git_to_spine_map(&self) -> &HashMap<GitOid, ObjectId> {
        &self.git_to_spine
    }

    pub fn execute_export(&mut self) -> Result<ExportStats, GitExportError> {
        let mut stats = ExportStats::default();

        fs::create_dir_all(&self.target_git_dir)?;
        fs::create_dir_all(self.target_git_dir.join("objects"))?;
        fs::create_dir_all(self.target_git_dir.join("objects").join("pack"))?;
        fs::create_dir_all(self.target_git_dir.join("objects").join("info"))?;
        fs::create_dir_all(self.target_git_dir.join("refs").join("heads"))?;
        fs::create_dir_all(self.target_git_dir.join("refs").join("tags"))?;

        // Standard Git config
        let config_path = self.target_git_dir.join("config");
        let bare_str = if self.is_bare { "true" } else { "false" };
        let config_content = format!(
            "[core]\n\trepositoryformatversion = 0\n\tfilemode = false\n\tbare = {}\n\tlogallrefupdates = true\n\tsymlinks = false\n\tignorecase = true\n",
            bare_str
        );
        fs::write(config_path, config_content)?;

        let store = FileSystemObjectStore::new(&self.spine_dir, HashAlgorithm::Blake3, 3)?;

        // Discover all references from .spine/refs
        let mut ref_map = GitRefMap::default();
        let heads_dir = self.spine_dir.join("refs").join("heads");
        if heads_dir.exists() {
            for entry in fs::read_dir(heads_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_file() {
                    let branch_name = entry.file_name().to_string_lossy().to_string();
                    let content = fs::read_to_string(&path)?;
                    if let Ok(spine_id) = ObjectId::from_hex(content.trim()) {
                        let git_oid = self.export_commit_recursive(&store, &spine_id, &mut stats)?;
                        ref_map.branches.insert(branch_name, git_oid);
                        stats.branches_count += 1;
                    }
                }
            }
        }

        // Tags
        let tags_dir = self.spine_dir.join("refs").join("tags");
        if tags_dir.exists() {
            for entry in fs::read_dir(tags_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_file() {
                    let tag_name = entry.file_name().to_string_lossy().to_string();
                    let content = fs::read_to_string(&path)?;
                    if let Ok(spine_id) = ObjectId::from_hex(content.trim()) {
                        let git_oid = self.export_tag_or_target(&store, &spine_id, &mut stats)?;
                        ref_map.tags.insert(tag_name, git_oid);
                    }
                }
            }
        }

        // HEAD
        let head_path = self.spine_dir.join("HEAD");
        if head_path.exists() {
            let content = fs::read_to_string(head_path)?;
            let trimmed = content.trim();
            if let Some(target) = trimmed.strip_prefix("ref: ") {
                ref_map.head = Some(GitHead::Symbolic(target.trim().to_string()));
            } else if let Ok(spine_id) = ObjectId::from_hex(trimmed) {
                if let Some(&git_oid) = self.spine_to_git.get(&spine_id) {
                    ref_map.head = Some(GitHead::Detached(git_oid));
                }
            }
        } else if let Some((first_branch, _)) = ref_map.branches.iter().next() {
            ref_map.head = Some(GitHead::Symbolic(format!("refs/heads/{}", first_branch)));
        } else {
            ref_map.head = Some(GitHead::Symbolic("refs/heads/main".to_string()));
        }

        // Write refs into target .git
        write_git_refs(&self.target_git_dir, &ref_map)?;

        // If non-bare, checkout working tree from HEAD
        if let Some(ref wt) = self.worktree_dir {
            if let Some(GitHead::Symbolic(ref sym)) = ref_map.head {
                if let Some(bname) = sym.strip_prefix("refs/heads/") {
                    if let Some(&git_oid) = ref_map.branches.get(bname) {
                        if let Some(&spine_id) = self.git_to_spine.get(&git_oid) {
                            if let Ok(commit) = spine_history::get_commit(&store, &spine_id) {
                                self.checkout_spine_tree(&store, &commit.root_tree, wt)?;
                            }
                        }
                    }
                }
            }
        }

        Ok(stats)
    }

    fn export_blob(
        &mut self,
        store: &FileSystemObjectStore,
        id: &ObjectId,
        stats: &mut ExportStats,
    ) -> Result<GitOid, GitExportError> {
        if let Some(&existing) = self.spine_to_git.get(id) {
            return Ok(existing);
        }

        let (_obj_type, payload) = store.read_object(id)?;
        let (compressed, git_oid) = serialize_loose_object(GitObjectType::Blob, &payload)
            .map_err(|e| GitExportError::Io(io::Error::new(io::ErrorKind::Other, e.to_string())))?;

        self.write_loose_object(git_oid, &compressed)?;
        self.spine_to_git.insert(*id, git_oid);
        self.git_to_spine.insert(git_oid, *id);
        stats.blobs_count += 1;

        Ok(git_oid)
    }

    fn export_tree_recursive(
        &mut self,
        store: &FileSystemObjectStore,
        tree_id: &ObjectId,
        stats: &mut ExportStats,
    ) -> Result<GitOid, GitExportError> {
        if let Some(&existing) = self.spine_to_git.get(tree_id) {
            return Ok(existing);
        }

        let (obj_type, payload) = store.read_object(tree_id)?;
        let entries = deserialize_tree(&payload)?;

        let mut git_entries = Vec::with_capacity(entries.len());
        for entry in entries {
            let (git_mode, entry_git_oid) = match entry.mode {
                FileMode::Directory => {
                    let oid = self.export_tree_recursive(store, &entry.object_id, stats)?;
                    (0o040000u32, oid)
                }
                FileMode::Regular => {
                    let oid = self.export_blob(store, &entry.object_id, stats)?;
                    (0o100644u32, oid)
                }
                FileMode::Executable => {
                    let oid = self.export_blob(store, &entry.object_id, stats)?;
                    (0o100755u32, oid)
                }
                FileMode::Symlink => {
                    let oid = self.export_blob(store, &entry.object_id, stats)?;
                    (0o120000u32, oid)
                }
            };

            git_entries.push(GitTreeEntry {
                mode: git_mode,
                name: entry.name,
                oid: entry_git_oid,
            });
        }

        let serialized = serialize_git_tree(git_entries);
        let (compressed, tree_git_oid) = serialize_loose_object(GitObjectType::Tree, &serialized)
            .map_err(|e| GitExportError::Io(io::Error::new(io::ErrorKind::Other, e.to_string())))?;

        self.write_loose_object(tree_git_oid, &compressed)?;
        self.spine_to_git.insert(*tree_id, tree_git_oid);
        self.git_to_spine.insert(tree_git_oid, *tree_id);
        stats.trees_count += 1;

        Ok(tree_git_oid)
    }

    fn export_commit_recursive(
        &mut self,
        store: &FileSystemObjectStore,
        commit_id: &ObjectId,
        stats: &mut ExportStats,
    ) -> Result<GitOid, GitExportError> {
        if let Some(&existing) = self.spine_to_git.get(commit_id) {
            return Ok(existing);
        }

        let commit = spine_history::get_commit(store, commit_id)?;

        // Export parents first
        let mut git_parents = Vec::with_capacity(commit.parents.len());
        for parent_id in &commit.parents {
            let parent_git_oid = self.export_commit_recursive(store, parent_id, stats)?;
            git_parents.push(parent_git_oid);
        }

        // Export root tree
        let git_tree_oid = self.export_tree_recursive(store, &commit.root_tree, stats)?;

        let timestamp_secs = commit.timestamp_nanos / 1_000_000_000;
        let author_line = format!(
            "{} <{}> {} +0000",
            commit.author_name, commit.author_email, timestamp_secs
        );
        let committer_line = format!(
            "{} <{}> {} +0000",
            commit.committer_name, commit.committer_email, timestamp_secs
        );

        let git_commit = GitCommit {
            tree: git_tree_oid,
            parents: git_parents,
            author: author_line,
            committer: committer_line,
            gpgsig: None,
            message: commit.message,
        };

        let serialized = serialize_git_commit(&git_commit);
        let (compressed, commit_git_oid) =
            serialize_loose_object(GitObjectType::Commit, &serialized).map_err(|e| {
                GitExportError::Io(io::Error::new(io::ErrorKind::Other, e.to_string()))
            })?;

        self.write_loose_object(commit_git_oid, &compressed)?;
        self.spine_to_git.insert(*commit_id, commit_git_oid);
        self.git_to_spine.insert(commit_git_oid, *commit_id);
        stats.commits_count += 1;

        Ok(commit_git_oid)
    }

    fn export_tag_or_target(
        &mut self,
        store: &FileSystemObjectStore,
        id: &ObjectId,
        stats: &mut ExportStats,
    ) -> Result<GitOid, GitExportError> {
        if let Some(&existing) = self.spine_to_git.get(id) {
            return Ok(existing);
        }

        let (obj_type, payload) = store.read_object(id)?;
        if obj_type == ObjectType::Tag {
            let spine_tag = deserialize_annotated_tag(&payload)?;
            let target_git_oid = self.export_commit_recursive(store, &spine_tag.target_object, stats)?;

            let timestamp_secs = spine_tag.timestamp_nanos / 1_000_000_000;
            let tagger_line = format!(
                "{} <{}> {} +0000",
                spine_tag.tagger_name, spine_tag.tagger_email, timestamp_secs
            );

            let git_tag = GitAnnotatedTag {
                object: target_git_oid,
                target_type: "commit".to_string(),
                tag_name: spine_tag.tag_name,
                tagger: tagger_line,
                message: spine_tag.message,
            };

            let serialized = serialize_git_tag(&git_tag);
            let (compressed, tag_git_oid) =
                serialize_loose_object(GitObjectType::Tag, &serialized).map_err(|e| {
                    GitExportError::Io(io::Error::new(io::ErrorKind::Other, e.to_string()))
                })?;

            self.write_loose_object(tag_git_oid, &compressed)?;
            self.spine_to_git.insert(*id, tag_git_oid);
            self.git_to_spine.insert(tag_git_oid, *id);
            stats.tags_count += 1;
            Ok(tag_git_oid)
        } else if obj_type == ObjectType::Commit {
            self.export_commit_recursive(store, id, stats)
        } else {
            self.export_blob(store, id, stats)
        }
    }

    fn write_loose_object(
        &self,
        oid: GitOid,
        compressed_payload: &[u8],
    ) -> Result<(), GitExportError> {
        let hex = oid.to_hex();
        let (dir_name, file_name) = hex.split_at(2);
        let target_dir = self.target_git_dir.join("objects").join(dir_name);
        fs::create_dir_all(&target_dir)?;
        let file_path = target_dir.join(file_name);
        fs::write(file_path, compressed_payload)?;
        Ok(())
    }

    fn checkout_spine_tree(
        &self,
        store: &FileSystemObjectStore,
        tree_id: &ObjectId,
        dir: &Path,
    ) -> Result<(), GitExportError> {
        fs::create_dir_all(dir)?;
        let (_, payload) = store.read_object(tree_id)?;
        let entries = deserialize_tree(&payload)?;

        for entry in entries {
            let target = dir.join(&entry.name);
            match entry.mode {
                FileMode::Directory => {
                    self.checkout_spine_tree(store, &entry.object_id, &target)?;
                }
                FileMode::Regular | FileMode::Executable | FileMode::Symlink => {
                    if let Ok((_, blob_bytes)) = store.read_object(&entry.object_id) {
                        fs::write(&target, blob_bytes)?;
                    }
                }
            }
        }

        Ok(())
    }
}
