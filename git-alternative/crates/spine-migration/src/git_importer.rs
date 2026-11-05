use crate::git_format::{
    parse_git_commit, parse_git_tag, parse_git_tree, parse_loose_object, parse_packfile,
    read_git_refs, GitHead, GitObjectType, GitOid,
};
use crate::lfs_interop::GitLfsPointer;
use spine_core::{AnnotatedTag, Commit, FileMode, HashAlgorithm, ObjectId, ObjectType, TreeEntry};
use spine_format::{serialize_annotated_tag, serialize_commit, serialize_tree};
use spine_object_store::{FileSystemObjectStore, ObjectStore};
use std::collections::HashMap;
use std::fs::{self};
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum GitImportError {
    #[error("I/O error during Git import: {0}")]
    Io(#[from] io::Error),
    #[error(
        "Invalid Git repository path: neither .git directory nor bare repository found at {0}"
    )]
    NotAGitRepository(PathBuf),
    #[error("Git format parsing error: {0}")]
    GitFormat(String),
    #[error("Object store error: {0}")]
    ObjectStore(#[from] spine_object_store::ObjectStoreError),
    #[error("Format error: {0}")]
    Format(#[from] spine_format::FormatError),
    #[error("Unresolved dependency: referenced Git OID {0} not found in Git object set")]
    UnresolvedOid(GitOid),
}

#[derive(Debug, Clone, Default)]
pub struct ImportStats {
    pub blobs_count: usize,
    pub trees_count: usize,
    pub commits_count: usize,
    pub tags_count: usize,
    pub branches_count: usize,
    pub lfs_pointers_count: usize,
}

pub struct GitImporter {
    git_dir: PathBuf,
    spine_dir: PathBuf,
    worktree_dir: Option<PathBuf>,
    raw_objects: HashMap<GitOid, (GitObjectType, Vec<u8>)>,
    git_to_spine: HashMap<GitOid, ObjectId>,
    spine_to_git: HashMap<ObjectId, GitOid>,
}

impl GitImporter {
    /// Discovers and validates the Git repository at `source_path`.
    /// Handles both non-bare repositories containing a `.git` folder and bare repositories.
    pub fn new<P: AsRef<Path>, S: AsRef<Path>>(
        source_path: P,
        dest_spine_dir: S,
        worktree_dir: Option<PathBuf>,
    ) -> Result<Self, GitImportError> {
        let source = source_path.as_ref();
        let dot_git = source.join(".git");

        let git_dir = if dot_git.exists() && dot_git.is_dir() {
            dot_git
        } else if source.join("HEAD").exists() && source.join("objects").exists() {
            source.to_path_buf()
        } else {
            return Err(GitImportError::NotAGitRepository(source.to_path_buf()));
        };

        Ok(Self {
            git_dir,
            spine_dir: dest_spine_dir.as_ref().to_path_buf(),
            worktree_dir,
            raw_objects: HashMap::new(),
            git_to_spine: HashMap::new(),
            spine_to_git: HashMap::new(),
        })
    }

    pub fn git_to_spine_map(&self) -> &HashMap<GitOid, ObjectId> {
        &self.git_to_spine
    }

    pub fn spine_to_git_map(&self) -> &HashMap<ObjectId, GitOid> {
        &self.spine_to_git
    }

    /// Ingests all loose objects and packfiles into memory.
    pub fn load_git_objects(&mut self) -> Result<(), GitImportError> {
        let objects_dir = self.git_dir.join("objects");
        if !objects_dir.exists() {
            return Ok(());
        }

        // 1. Ingest loose objects from objects/??/*
        for entry in fs::read_dir(&objects_dir)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();

            if path.is_dir() && name.len() == 2 && hex::decode(&name).is_ok() {
                for obj_entry in fs::read_dir(&path)? {
                    let obj_entry = obj_entry?;
                    let obj_path = obj_entry.path();
                    let obj_name = obj_entry.file_name().to_string_lossy().to_string();

                    if obj_path.is_file() && obj_name.len() == 38 {
                        let full_hex = format!("{}{}", name, obj_name);
                        if let Ok(expected_oid) = GitOid::from_hex(&full_hex) {
                            let compressed = fs::read(&obj_path)?;
                            if let Ok((obj_type, payload, computed_oid)) =
                                parse_loose_object(&compressed, Some(expected_oid))
                            {
                                self.raw_objects.insert(computed_oid, (obj_type, payload));
                            }
                        }
                    }
                }
            }
        }

        // 2. Ingest packfiles from objects/pack/*.pack
        let pack_dir = objects_dir.join("pack");
        if pack_dir.exists() && pack_dir.is_dir() {
            for entry in fs::read_dir(&pack_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_file() && path.extension().is_some_and(|ext| ext == "pack") {
                    let pack_data = fs::read(&path)?;
                    if let Ok(pack_objects) = parse_packfile(&pack_data) {
                        for obj in pack_objects {
                            self.raw_objects
                                .insert(obj.oid, (obj.obj_type, obj.payload));
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Converts all loaded Git objects into Spine CAS format and writes references.
    pub fn execute_import(&mut self) -> Result<ImportStats, GitImportError> {
        let mut stats = ImportStats::default();

        let store = FileSystemObjectStore::new(&self.spine_dir, HashAlgorithm::Blake3, 3)?;

        // Ensure directories exist
        fs::create_dir_all(self.spine_dir.join("refs").join("heads"))?;
        fs::create_dir_all(self.spine_dir.join("refs").join("tags"))?;

        // 1. First Pass: Import all Blobs
        for (&oid, (obj_type, payload)) in &self.raw_objects {
            if *obj_type == GitObjectType::Blob {
                if GitLfsPointer::is_lfs_pointer(payload) {
                    stats.lfs_pointers_count += 1;
                }
                let spine_id = store.write_object(ObjectType::Blob, payload)?;
                self.git_to_spine.insert(oid, spine_id);
                self.spine_to_git.insert(spine_id, oid);
                stats.blobs_count += 1;
            }
        }

        // 2. Second Pass: Import Trees (post-order / bottom-up)
        let tree_oids: Vec<GitOid> = self
            .raw_objects
            .iter()
            .filter(|(_, (t, _))| *t == GitObjectType::Tree)
            .map(|(&id, _)| id)
            .collect();

        for oid in tree_oids {
            self.import_tree_recursive(oid, &store, &mut stats)?;
        }

        // 3. Third Pass: Import Commits (in topological / depth order)
        let commit_oids: Vec<GitOid> = self
            .raw_objects
            .iter()
            .filter(|(_, (t, _))| *t == GitObjectType::Commit)
            .map(|(&id, _)| id)
            .collect();

        for oid in commit_oids {
            self.import_commit_recursive(oid, &store, &mut stats)?;
        }

        // 4. Fourth Pass: Import Annotated Tags
        let tag_oids: Vec<GitOid> = self
            .raw_objects
            .iter()
            .filter(|(_, (t, _))| *t == GitObjectType::Tag)
            .map(|(&id, _)| id)
            .collect();

        for oid in tag_oids {
            let (_, payload) = self.raw_objects.get(&oid).unwrap();
            let git_tag =
                parse_git_tag(payload).map_err(|e| GitImportError::GitFormat(e.to_string()))?;

            let target_spine_oid = self
                .git_to_spine
                .get(&git_tag.object)
                .copied()
                .ok_or(GitImportError::UnresolvedOid(git_tag.object))?;

            let target_type = match git_tag.target_type.as_str() {
                "commit" => ObjectType::Commit,
                "tree" => ObjectType::Tree,
                "blob" => ObjectType::Blob,
                "tag" => ObjectType::Tag,
                _ => ObjectType::Commit,
            };

            let (tagger_name, tagger_email, timestamp) =
                crate::git_format::commit::parse_identity_line(&git_tag.tagger);

            let spine_tag = AnnotatedTag::new(
                target_spine_oid,
                target_type,
                git_tag.tag_name,
                tagger_name,
                tagger_email,
                timestamp * 1_000_000_000,
                git_tag.message,
            );

            let serialized = serialize_annotated_tag(&spine_tag);
            let spine_id = store.write_object(ObjectType::Tag, &serialized)?;
            self.git_to_spine.insert(oid, spine_id);
            self.spine_to_git.insert(spine_id, oid);
            stats.tags_count += 1;
        }

        // 5. Fifth Pass: Import References
        let refs =
            read_git_refs(&self.git_dir).map_err(|e| GitImportError::GitFormat(e.to_string()))?;

        // Write branches
        for (branch_name, git_oid) in &refs.branches {
            if let Some(&spine_oid) = self.git_to_spine.get(git_oid) {
                let ref_path = self.spine_dir.join("refs").join("heads").join(branch_name);
                if let Some(parent) = ref_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(ref_path, format!("{}\n", spine_oid))?;
                stats.branches_count += 1;
            }
        }

        // Write tags
        for (tag_name, git_oid) in &refs.tags {
            if let Some(&spine_oid) = self.git_to_spine.get(git_oid) {
                let ref_path = self.spine_dir.join("refs").join("tags").join(tag_name);
                if let Some(parent) = ref_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(ref_path, format!("{}\n", spine_oid))?;
            }
        }

        // Write HEAD
        if let Some(head) = refs.head {
            let head_path = self.spine_dir.join("HEAD");
            match head {
                GitHead::Symbolic(target) => {
                    fs::write(head_path, format!("ref: {}\n", target))?;
                }
                GitHead::Detached(git_oid) => {
                    if let Some(&spine_oid) = self.git_to_spine.get(&git_oid) {
                        fs::write(head_path, format!("{}\n", spine_oid))?;
                    }
                }
            }
        } else {
            // Default to refs/heads/main
            fs::write(self.spine_dir.join("HEAD"), "ref: refs/heads/main\n")?;
        }

        // Checkout working copy if worktree directory is present
        if let Some(ref wt) = self.worktree_dir {
            self.checkout_head(&store, wt)?;
        }

        Ok(stats)
    }

    fn import_tree_recursive(
        &mut self,
        oid: GitOid,
        store: &FileSystemObjectStore,
        stats: &mut ImportStats,
    ) -> Result<ObjectId, GitImportError> {
        if let Some(&existing) = self.git_to_spine.get(&oid) {
            return Ok(existing);
        }

        let (_, payload) = self
            .raw_objects
            .get(&oid)
            .ok_or(GitImportError::UnresolvedOid(oid))?
            .clone();

        let entries =
            parse_git_tree(&payload).map_err(|e| GitImportError::GitFormat(e.to_string()))?;

        let mut spine_entries = Vec::with_capacity(entries.len());

        for entry in entries {
            let spine_mode = if entry.is_tree() {
                FileMode::Directory
            } else if entry.is_executable() {
                FileMode::Executable
            } else if entry.is_symlink() {
                FileMode::Symlink
            } else {
                FileMode::Regular
            };

            let entry_spine_id = if entry.is_tree() {
                self.import_tree_recursive(entry.oid, store, stats)?
            } else {
                *self
                    .git_to_spine
                    .get(&entry.oid)
                    .ok_or(GitImportError::UnresolvedOid(entry.oid))?
            };

            spine_entries.push(TreeEntry {
                mode: spine_mode,
                name: entry.name,
                object_id: entry_spine_id,
            });
        }

        let serialized = serialize_tree(&spine_entries);
        let spine_tree_id = store.write_object(ObjectType::Tree, &serialized)?;
        self.git_to_spine.insert(oid, spine_tree_id);
        self.spine_to_git.insert(spine_tree_id, oid);
        stats.trees_count += 1;

        Ok(spine_tree_id)
    }

    fn import_commit_recursive(
        &mut self,
        oid: GitOid,
        store: &FileSystemObjectStore,
        stats: &mut ImportStats,
    ) -> Result<ObjectId, GitImportError> {
        if let Some(&existing) = self.git_to_spine.get(&oid) {
            return Ok(existing);
        }

        let (_, payload) = self
            .raw_objects
            .get(&oid)
            .ok_or(GitImportError::UnresolvedOid(oid))?
            .clone();

        let git_commit =
            parse_git_commit(&payload).map_err(|e| GitImportError::GitFormat(e.to_string()))?;

        // Recursively resolve parents first
        let mut spine_parents = Vec::with_capacity(git_commit.parents.len());
        for parent_oid in &git_commit.parents {
            let parent_spine_id = self.import_commit_recursive(*parent_oid, store, stats)?;
            spine_parents.push(parent_spine_id);
        }

        // Resolve root tree
        let root_tree_id = self.import_tree_recursive(git_commit.tree, store, stats)?;

        let (author_name, author_email, timestamp) = git_commit.parse_author();
        let (committer_name, committer_email, _) = git_commit.parse_committer();

        let spine_commit = Commit::new(
            root_tree_id,
            spine_parents,
            author_name,
            author_email,
            committer_name,
            committer_email,
            timestamp * 1_000_000_000,
            git_commit.message,
        );

        let serialized = serialize_commit(&spine_commit);
        let spine_commit_id = store.write_object(ObjectType::Commit, &serialized)?;
        self.git_to_spine.insert(oid, spine_commit_id);
        self.spine_to_git.insert(spine_commit_id, oid);
        stats.commits_count += 1;

        Ok(spine_commit_id)
    }

    fn checkout_head(
        &self,
        store: &FileSystemObjectStore,
        worktree_path: &Path,
    ) -> Result<(), GitImportError> {
        let head_path = self.spine_dir.join("HEAD");
        if !head_path.exists() {
            return Ok(());
        }
        let content = fs::read_to_string(head_path)?;
        let trimmed = content.trim();

        let commit_oid = if let Some(target) = trimmed.strip_prefix("ref: ") {
            let ref_file = self.spine_dir.join(target);
            if ref_file.exists() {
                let id_str = fs::read_to_string(ref_file)?;
                ObjectId::from_hex(id_str.trim()).ok()
            } else {
                None
            }
        } else {
            ObjectId::from_hex(trimmed).ok()
        };

        if let Some(cid) = commit_oid {
            if let Ok(commit) = spine_history::get_commit(store, &cid) {
                self.checkout_tree(store, &commit.root_tree, worktree_path)?;
            }
        }

        Ok(())
    }

    fn checkout_tree(
        &self,
        store: &FileSystemObjectStore,
        tree_id: &ObjectId,
        dir: &Path,
    ) -> Result<(), GitImportError> {
        fs::create_dir_all(dir)?;
        let (obj_type, payload) = store.read_object(tree_id)?;
        if obj_type != ObjectType::Tree {
            return Ok(());
        }
        let entries = spine_format::deserialize_tree(&payload)?;

        for entry in entries {
            let target = dir.join(&entry.name);
            match entry.mode {
                FileMode::Directory => {
                    self.checkout_tree(store, &entry.object_id, &target)?;
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
