use crate::core::error::ForgeError;
use crate::core::types::{DiffFileDto, MergeResultDto, MergeabilityReport, PullRequestDiffDto};
use serde::Serialize;
use spine_core::{Commit, FileMode, HashAlgorithm, ObjectId, ObjectType, TreeEntry};
use spine_diff::{compare_trees, compute_line_diff, format_unified_diff, is_binary, DiffOp, TreeChange};
use spine_format::{deserialize_commit, deserialize_tree, serialize_commit, serialize_tree};
use spine_history::{find_merge_bases, get_commit, is_ancestor};
use spine_merge::{merge_3way_trees, ResolvedPath};
use spine_object_store::{CompositeObjectStore, ObjectStore};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize)]
pub struct BranchDto {
    pub name: String,
    pub commit_id: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct TagDto {
    pub name: String,
    pub target_id: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct CommitDto {
    pub id: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    pub timestamp_nanos: i64,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct TreeEntryDto {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub object_id: String,
    pub mode: u32,
}

#[derive(Clone, Debug, Serialize)]
pub struct BlobContentDto {
    pub path: String,
    pub size: usize,
    pub is_binary: bool,
    pub content: Option<String>,
    pub object_id: String,
}

pub struct NativeVcsService {
    storage_root: PathBuf,
}

impl NativeVcsService {
    pub fn new<P: AsRef<Path>>(storage_root: P) -> Self {
        Self {
            storage_root: storage_root.as_ref().to_path_buf(),
        }
    }

    fn repo_spine_dir(&self, native_repo_id: &str) -> Result<PathBuf, ForgeError> {
        let path = self
            .storage_root
            .join("repos")
            .join(native_repo_id)
            .join(".spine");
        if !path.exists() {
            return Err(ForgeError::NotFound(
                "Repository data not found on storage".into(),
            ));
        }
        Ok(path)
    }

    fn open_store(&self, native_repo_id: &str) -> Result<CompositeObjectStore, ForgeError> {
        let spine_dir = self.repo_spine_dir(native_repo_id)?;
        CompositeObjectStore::new(&spine_dir, HashAlgorithm::Blake3, 3)
            .map_err(|e| ForgeError::Internal(format!("Failed to open object store: {}", e)))
    }

    pub fn list_branches(&self, native_repo_id: &str) -> Result<Vec<BranchDto>, ForgeError> {
        let heads_dir = self.repo_spine_dir(native_repo_id)?.join("refs").join("heads");
        if !heads_dir.exists() {
            return Ok(Vec::new());
        }

        let mut branches = Vec::new();
        if let Ok(entries) = fs::read_dir(&heads_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if let Ok(commit_id) = fs::read_to_string(&p) {
                        branches.push(BranchDto {
                            name,
                            commit_id: commit_id.trim().to_string(),
                        });
                    }
                }
            }
        }
        branches.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(branches)
    }

    pub fn list_tags(&self, native_repo_id: &str) -> Result<Vec<TagDto>, ForgeError> {
        let tags_dir = self.repo_spine_dir(native_repo_id)?.join("refs").join("tags");
        if !tags_dir.exists() {
            return Ok(Vec::new());
        }

        let mut tags = Vec::new();
        if let Ok(entries) = fs::read_dir(&tags_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_file() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if let Ok(target_id) = fs::read_to_string(&p) {
                        tags.push(TagDto {
                            name,
                            target_id: target_id.trim().to_string(),
                        });
                    }
                }
            }
        }
        tags.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(tags)
    }

    pub fn resolve_ref(&self, native_repo_id: &str, ref_name: &str) -> Result<ObjectId, ForgeError> {
        let trimmed = ref_name.trim();

        // 1. Direct hex commit ID
        if trimmed.len() == 64 {
            if let Ok(oid) = ObjectId::from_hex(trimmed) {
                return Ok(oid);
            }
        }

        let spine_dir = self.repo_spine_dir(native_repo_id)?;

        // 2. Branch ref
        let branch_file = spine_dir.join("refs").join("heads").join(trimmed);
        if branch_file.exists() {
            let content = fs::read_to_string(&branch_file)
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
            return ObjectId::from_hex(content.trim())
                .map_err(|e| ForgeError::Internal(format!("Invalid branch commit id: {}", e)));
        }

        // 3. Tag ref
        let tag_file = spine_dir.join("refs").join("tags").join(trimmed);
        if tag_file.exists() {
            let content = fs::read_to_string(&tag_file)
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
            return ObjectId::from_hex(content.trim())
                .map_err(|e| ForgeError::Internal(format!("Invalid tag commit id: {}", e)));
        }

        Err(ForgeError::NotFound(format!("Ref '{}' not found", trimmed)))
    }

    pub fn get_commits(
        &self,
        native_repo_id: &str,
        ref_name: Option<&str>,
        limit: usize,
    ) -> Result<Vec<CommitDto>, ForgeError> {
        let r_name = ref_name.unwrap_or("main");
        let start_oid = match self.resolve_ref(native_repo_id, r_name) {
            Ok(oid) => oid,
            Err(_) => {
                // Return empty if repo has no commits yet
                return Ok(Vec::new());
            }
        };

        let store = self.open_store(native_repo_id)?;
        let mut commits = Vec::new();
        let mut queue = vec![start_oid];
        let mut visited = std::collections::HashSet::new();

        while let Some(oid) = queue.pop() {
            if !visited.insert(oid) {
                continue;
            }

            match store.read_object(&oid) {
                Ok((ObjectType::Commit, payload)) => {
                    let commit = deserialize_commit(&payload)
                        .map_err(|e| ForgeError::Internal(format!("Commit format error: {}", e)))?;

                    commits.push(CommitDto {
                        id: oid.to_hex(),
                        parents: commit.parents.iter().map(|p| p.to_hex()).collect(),
                        author_name: commit.author_name,
                        author_email: commit.author_email,
                        timestamp_nanos: commit.timestamp_nanos,
                        message: commit.message,
                    });

                    if commits.len() >= limit {
                        break;
                    }

                    for parent in commit.parents.into_iter().rev() {
                        queue.push(parent);
                    }
                }
                _ => break,
            }
        }

        Ok(commits)
    }

    pub fn get_tree(
        &self,
        native_repo_id: &str,
        ref_name: &str,
        subpath: Option<&str>,
    ) -> Result<Vec<TreeEntryDto>, ForgeError> {
        let commit_oid = self.resolve_ref(native_repo_id, ref_name)?;
        let store = self.open_store(native_repo_id)?;

        let (obj_type, payload) = store
            .read_object(&commit_oid)
            .map_err(|e| ForgeError::NotFound(format!("Commit object not found: {}", e)))?;

        if obj_type != ObjectType::Commit {
            return Err(ForgeError::BadRequest("Target is not a commit".into()));
        }

        let commit = deserialize_commit(&payload)
            .map_err(|e| ForgeError::Internal(format!("Format error: {}", e)))?;

        let mut current_tree_oid = commit.root_tree;

        // Traverse subpath if provided
        let norm_subpath = subpath
            .map(|s| s.trim_matches('/'))
            .unwrap_or("");

        if !norm_subpath.is_empty() {
            let segments: Vec<&str> = norm_subpath.split('/').filter(|s| !s.is_empty()).collect();
            for seg in segments {
                // Prevent path traversal
                if seg == ".." || seg == "." {
                    return Err(ForgeError::BadRequest("Invalid path segment".into()));
                }

                let (t_type, t_payload) = store
                    .read_object(&current_tree_oid)
                    .map_err(|e| ForgeError::NotFound(format!("Tree object not found: {}", e)))?;
                if t_type != ObjectType::Tree {
                    return Err(ForgeError::BadRequest("Expected tree object".into()));
                }

                let entries = deserialize_tree(&t_payload)
                    .map_err(|e| ForgeError::Internal(format!("Tree parse error: {}", e)))?;

                let match_entry = entries
                    .into_iter()
                    .find(|e| e.name == seg && e.mode == FileMode::Directory);

                match match_entry {
                    Some(e) => current_tree_oid = e.object_id,
                    None => return Err(ForgeError::NotFound(format!("Path '{}' not found", norm_subpath))),
                }
            }
        }

        // Read current tree entries
        let (t_type, t_payload) = store
            .read_object(&current_tree_oid)
            .map_err(|e| ForgeError::NotFound(format!("Tree object not found: {}", e)))?;
        if t_type != ObjectType::Tree {
            return Err(ForgeError::BadRequest("Expected tree object".into()));
        }

        let entries = deserialize_tree(&t_payload)
            .map_err(|e| ForgeError::Internal(format!("Tree parse error: {}", e)))?;

        let mut dlist = Vec::new();
        for e in entries {
            let is_dir = e.mode == FileMode::Directory;
            let full_path = if norm_subpath.is_empty() {
                e.name.clone()
            } else {
                format!("{}/{}", norm_subpath, e.name)
            };
            dlist.push(TreeEntryDto {
                name: e.name,
                path: full_path,
                is_dir,
                object_id: e.object_id.to_hex(),
                mode: e.mode.as_u32(),
            });
        }

        // Sort: directories first, then alphabetical
        dlist.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        Ok(dlist)
    }

    pub fn get_blob(
        &self,
        native_repo_id: &str,
        ref_name: &str,
        file_path: &str,
    ) -> Result<BlobContentDto, ForgeError> {
        let norm_path = file_path.trim_matches('/');
        let segments: Vec<&str> = norm_path.split('/').filter(|s| !s.is_empty()).collect();
        if segments.is_empty() {
            return Err(ForgeError::BadRequest("Empty file path".into()));
        }

        let commit_oid = self.resolve_ref(native_repo_id, ref_name)?;
        let store = self.open_store(native_repo_id)?;

        let (_, commit_payload) = store
            .read_object(&commit_oid)
            .map_err(|e| ForgeError::NotFound(format!("Commit not found: {}", e)))?;
        let commit = deserialize_commit(&commit_payload)
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let mut current_tree_oid = commit.root_tree;

        for (i, seg) in segments.iter().enumerate() {
            if *seg == ".." || *seg == "." {
                return Err(ForgeError::BadRequest("Invalid path segment".into()));
            }

            let (_, t_payload) = store
                .read_object(&current_tree_oid)
                .map_err(|e| ForgeError::NotFound(format!("Tree not found: {}", e)))?;
            let entries = deserialize_tree(&t_payload)
                .map_err(|e| ForgeError::Internal(e.to_string()))?;

            let is_last = i == segments.len() - 1;

            if is_last {
                // Find file blob
                let blob_entry = entries
                    .into_iter()
                    .find(|e| e.name == *seg && e.mode != FileMode::Directory)
                    .ok_or_else(|| ForgeError::NotFound(format!("File '{}' not found", norm_path)))?;

                let (b_type, b_payload) = store
                    .read_object(&blob_entry.object_id)
                    .map_err(|e| ForgeError::NotFound(format!("Blob not found: {}", e)))?;

                if b_type != ObjectType::Blob {
                    return Err(ForgeError::BadRequest("Target is not a blob".into()));
                }

                let size = b_payload.len();
                let is_binary = b_payload.contains(&0u8);

                let content = if !is_binary {
                    String::from_utf8(b_payload).ok()
                } else {
                    None
                };

                return Ok(BlobContentDto {
                    path: norm_path.to_string(),
                    size,
                    is_binary,
                    content,
                    object_id: blob_entry.object_id.to_hex(),
                });
            } else {
                // Find intermediate directory tree
                let dir_entry = entries
                    .into_iter()
                    .find(|e| e.name == *seg && e.mode == FileMode::Directory)
                    .ok_or_else(|| ForgeError::NotFound(format!("Directory '{}' not found", seg)))?;
                current_tree_oid = dir_entry.object_id;
            }
        }

        Err(ForgeError::NotFound("File not found".into()))
    }

    /// Recursively flattens an object tree into a map of full-path -> (FileMode, ObjectId).
    pub fn flatten_tree<S: ObjectStore>(
        &self,
        store: &S,
        tree_oid: &ObjectId,
        prefix: &str,
        out: &mut BTreeMap<String, (FileMode, ObjectId)>,
    ) -> Result<(), ForgeError> {
        let (obj_type, payload) = store
            .read_object(tree_oid)
            .map_err(|e| ForgeError::Internal(format!("Tree read failed: {}", e)))?;
        if obj_type != ObjectType::Tree {
            return Ok(());
        }
        let entries = deserialize_tree(&payload)
            .map_err(|e| ForgeError::Internal(format!("Tree deserialize error: {}", e)))?;
        for entry in entries {
            let path = if prefix.is_empty() {
                entry.name.clone()
            } else {
                format!("{}/{}", prefix, entry.name)
            };
            if entry.mode == FileMode::Directory {
                self.flatten_tree(store, &entry.object_id, &path, out)?;
            } else {
                out.insert(path, (entry.mode, entry.object_id));
            }
        }
        Ok(())
    }

    /// Recursively converts a flattened path map into nested Tree objects written to the store.
    pub fn write_tree_recursive<S: ObjectStore>(
        &self,
        store: &S,
        entries_map: &BTreeMap<String, (FileMode, ObjectId)>,
    ) -> Result<ObjectId, ForgeError> {
        let mut direct_entries = Vec::new();
        let mut subdirs: BTreeMap<String, BTreeMap<String, (FileMode, ObjectId)>> = BTreeMap::new();

        for (path, (mode, oid)) in entries_map {
            if let Some((dir, rest)) = path.split_once('/') {
                subdirs
                    .entry(dir.to_string())
                    .or_default()
                    .insert(rest.to_string(), (*mode, *oid));
            } else {
                direct_entries.push(TreeEntry {
                    mode: *mode,
                    name: path.clone(),
                    object_id: *oid,
                });
            }
        }

        for (dir_name, sub_map) in subdirs {
            let sub_tree_oid = self.write_tree_recursive(store, &sub_map)?;
            direct_entries.push(TreeEntry {
                mode: FileMode::Directory,
                name: dir_name,
                object_id: sub_tree_oid,
            });
        }

        let payload = serialize_tree(&direct_entries);
        store
            .write_object(ObjectType::Tree, &payload)
            .map_err(|e| ForgeError::Internal(format!("Failed to write tree object: {}", e)))
    }

    /// Computes full branch comparison and unified diff for Pull Requests.
    pub fn compare_branches(
        &self,
        native_repo_id: &str,
        base_ref: &str,
        head_ref: &str,
    ) -> Result<PullRequestDiffDto, ForgeError> {
        let base_oid = self.resolve_ref(native_repo_id, base_ref)?;
        let head_oid = self.resolve_ref(native_repo_id, head_ref)?;
        let store = self.open_store(native_repo_id)?;

        let merge_bases = find_merge_bases(&store, &base_oid, &head_oid)
            .map_err(|e| ForgeError::Internal(format!("Failed to calculate merge base: {}", e)))?;
        let merge_base_oid = merge_bases.first().cloned().unwrap_or(base_oid);

        let base_commit = get_commit(&store, &merge_base_oid)
            .map_err(|e| ForgeError::Internal(format!("Failed to read base commit: {}", e)))?;
        let head_commit = get_commit(&store, &head_oid)
            .map_err(|e| ForgeError::Internal(format!("Failed to read head commit: {}", e)))?;

        let mut base_map = BTreeMap::new();
        self.flatten_tree(&store, &base_commit.root_tree, "", &mut base_map)?;

        let mut head_map = BTreeMap::new();
        self.flatten_tree(&store, &head_commit.root_tree, "", &mut head_map)?;

        let tree_changes = compare_trees(&base_map, &head_map);

        let mut files = Vec::new();
        let mut total_additions = 0;
        let mut total_deletions = 0;

        for change in tree_changes {
            match change {
                TreeChange::Added { path, object_id } => {
                    let (_, blob_bytes) = store
                        .read_object(&object_id)
                        .map_err(|e| ForgeError::Internal(e.to_string()))?;
                    let bin = is_binary(&blob_bytes);
                    let (patch, adds, dels) = if !bin {
                        let text = String::from_utf8_lossy(&blob_bytes);
                        let ops = compute_line_diff("", &text);
                        let adds_cnt = ops
                            .iter()
                            .filter(|op| matches!(op, DiffOp::Addition(_)))
                            .count();
                        let p = format_unified_diff(&path, "", &text);
                        (p, adds_cnt, 0)
                    } else {
                        ("(Binary file added)".to_string(), 0, 0)
                    };
                    total_additions += adds;
                    total_deletions += dels;
                    files.push(DiffFileDto {
                        path,
                        old_path: None,
                        change_type: "ADDED".to_string(),
                        is_binary: bin,
                        patch,
                        additions: adds,
                        deletions: dels,
                    });
                }
                TreeChange::Deleted { path, object_id } => {
                    let (_, blob_bytes) = store
                        .read_object(&object_id)
                        .map_err(|e| ForgeError::Internal(e.to_string()))?;
                    let bin = is_binary(&blob_bytes);
                    let (patch, adds, dels) = if !bin {
                        let text = String::from_utf8_lossy(&blob_bytes);
                        let ops = compute_line_diff(&text, "");
                        let dels_cnt = ops
                            .iter()
                            .filter(|op| matches!(op, DiffOp::Deletion(_)))
                            .count();
                        let p = format_unified_diff(&path, &text, "");
                        (p, 0, dels_cnt)
                    } else {
                        ("(Binary file removed)".to_string(), 0, 0)
                    };
                    total_additions += adds;
                    total_deletions += dels;
                    files.push(DiffFileDto {
                        path: path.clone(),
                        old_path: Some(path),
                        change_type: "DELETED".to_string(),
                        is_binary: bin,
                        patch,
                        additions: adds,
                        deletions: dels,
                    });
                }
                TreeChange::Modified {
                    path,
                    old_id,
                    new_id,
                } => {
                    let (_, old_bytes) = store
                        .read_object(&old_id)
                        .map_err(|e| ForgeError::Internal(e.to_string()))?;
                    let (_, new_bytes) = store
                        .read_object(&new_id)
                        .map_err(|e| ForgeError::Internal(e.to_string()))?;
                    let bin = is_binary(&old_bytes) || is_binary(&new_bytes);
                    let (patch, adds, dels) = if !bin {
                        let old_text = String::from_utf8_lossy(&old_bytes);
                        let new_text = String::from_utf8_lossy(&new_bytes);
                        let ops = compute_line_diff(&old_text, &new_text);
                        let adds_cnt = ops
                            .iter()
                            .filter(|op| matches!(op, DiffOp::Addition(_)))
                            .count();
                        let dels_cnt = ops
                            .iter()
                            .filter(|op| matches!(op, DiffOp::Deletion(_)))
                            .count();
                        let p = format_unified_diff(&path, &old_text, &new_text);
                        (p, adds_cnt, dels_cnt)
                    } else {
                        ("(Binary file modified)".to_string(), 0, 0)
                    };
                    total_additions += adds;
                    total_deletions += dels;
                    files.push(DiffFileDto {
                        path: path.clone(),
                        old_path: Some(path),
                        change_type: "MODIFIED".to_string(),
                        is_binary: bin,
                        patch,
                        additions: adds,
                        deletions: dels,
                    });
                }
            }
        }

        // Count commits from head to merge_base
        let mut commits_count = 0;
        let mut queue = vec![head_oid];
        let mut visited = std::collections::HashSet::new();
        while let Some(cid) = queue.pop() {
            if cid == merge_base_oid || !visited.insert(cid) {
                continue;
            }
            commits_count += 1;
            if let Ok(c) = get_commit(&store, &cid) {
                for p in c.parents {
                    queue.push(p);
                }
            }
        }

        Ok(PullRequestDiffDto {
            base_commit_id: base_oid.to_hex(),
            head_commit_id: head_oid.to_hex(),
            merge_base_commit_id: merge_base_oid.to_hex(),
            files,
            total_additions,
            total_deletions,
            commits_count,
        })
    }

    /// Evaluates mergeability dry-run and returns conflict files if any.
    pub fn check_mergeability(
        &self,
        native_repo_id: &str,
        base_ref: &str,
        head_ref: &str,
    ) -> Result<MergeabilityReport, ForgeError> {
        let base_oid = self.resolve_ref(native_repo_id, base_ref)?;
        let head_oid = self.resolve_ref(native_repo_id, head_ref)?;
        let store = self.open_store(native_repo_id)?;

        if base_oid == head_oid {
            return Ok(MergeabilityReport {
                mergeable: true,
                status: "UP_TO_DATE".to_string(),
                conflict_files: Vec::new(),
                base_commit_id: base_oid.to_hex(),
                head_commit_id: head_oid.to_hex(),
            });
        }

        if is_ancestor(&store, &base_oid, &head_oid).unwrap_or(false) {
            // Fast-forwardable
            return Ok(MergeabilityReport {
                mergeable: true,
                status: "CLEAN".to_string(),
                conflict_files: Vec::new(),
                base_commit_id: base_oid.to_hex(),
                head_commit_id: head_oid.to_hex(),
            });
        }

        let merge_bases = find_merge_bases(&store, &base_oid, &head_oid)
            .map_err(|e| ForgeError::Internal(format!("Failed to calculate merge base: {}", e)))?;
        let merge_base_oid = merge_bases.first().cloned().unwrap_or(base_oid);

        let base_commit = get_commit(&store, &base_oid)
            .map_err(|e| ForgeError::Internal(format!("Failed to read base commit: {}", e)))?;
        let head_commit = get_commit(&store, &head_oid)
            .map_err(|e| ForgeError::Internal(format!("Failed to read head commit: {}", e)))?;
        let mb_commit = get_commit(&store, &merge_base_oid)
            .map_err(|e| ForgeError::Internal(format!("Failed to read merge base commit: {}", e)))?;

        let mut base_map = BTreeMap::new();
        self.flatten_tree(&store, &base_commit.root_tree, "", &mut base_map)?;

        let mut head_map = BTreeMap::new();
        self.flatten_tree(&store, &head_commit.root_tree, "", &mut head_map)?;

        let mut mb_map = BTreeMap::new();
        self.flatten_tree(&store, &mb_commit.root_tree, "", &mut mb_map)?;

        let resolutions = merge_3way_trees(&store, &mb_map, &base_map, &head_map)
            .map_err(|e| ForgeError::Internal(format!("3-way merge check failed: {}", e)))?;

        let mut conflict_files = Vec::new();
        for (path, res) in resolutions {
            if matches!(res, ResolvedPath::Conflict(_)) {
                conflict_files.push(path);
            }
        }

        let mergeable = conflict_files.is_empty();
        let status = if mergeable { "CLEAN" } else { "CONFLICT" }.to_string();

        Ok(MergeabilityReport {
            mergeable,
            status,
            conflict_files,
            base_commit_id: base_oid.to_hex(),
            head_commit_id: head_oid.to_hex(),
        })
    }

    /// Performs atomic 3-way merge, creates merge commit, and updates target branch ref.
    pub fn execute_merge(
        &self,
        native_repo_id: &str,
        base_branch: &str,
        head_branch: &str,
        author_name: &str,
        author_email: &str,
        commit_message: &str,
    ) -> Result<MergeResultDto, ForgeError> {
        let base_oid = self.resolve_ref(native_repo_id, base_branch)?;
        let head_oid = self.resolve_ref(native_repo_id, head_branch)?;
        let store = self.open_store(native_repo_id)?;

        let merge_bases = find_merge_bases(&store, &base_oid, &head_oid)
            .map_err(|e| ForgeError::Internal(format!("Merge base error: {}", e)))?;
        let merge_base_oid = merge_bases.first().cloned().unwrap_or(base_oid);

        let base_commit = get_commit(&store, &base_oid)
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let head_commit = get_commit(&store, &head_oid)
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mb_commit = get_commit(&store, &merge_base_oid)
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let mut base_map = BTreeMap::new();
        self.flatten_tree(&store, &base_commit.root_tree, "", &mut base_map)?;

        let mut head_map = BTreeMap::new();
        self.flatten_tree(&store, &head_commit.root_tree, "", &mut head_map)?;

        let mut mb_map = BTreeMap::new();
        self.flatten_tree(&store, &mb_commit.root_tree, "", &mut mb_map)?;

        let resolutions = merge_3way_trees(&store, &mb_map, &base_map, &head_map)
            .map_err(|e| ForgeError::Internal(format!("Merge failed: {}", e)))?;

        let mut merged_map = BTreeMap::new();
        let mut conflicts = Vec::new();

        for (path, res) in resolutions {
            match res {
                ResolvedPath::KeepOurs => {
                    if let Some(entry) = base_map.get(&path) {
                        merged_map.insert(path, *entry);
                    }
                }
                ResolvedPath::TakeTheirs {
                    mode, object_id, ..
                } => {
                    merged_map.insert(path, (mode, object_id));
                }
                ResolvedPath::MergedClean { mode, content, .. } => {
                    let blob_oid = store
                        .write_object(ObjectType::Blob, &content)
                        .map_err(|e| {
                            ForgeError::Internal(format!("Failed to write merged blob: {}", e))
                        })?;
                    merged_map.insert(path, (mode, blob_oid));
                }
                ResolvedPath::Delete => {}
                ResolvedPath::Conflict(c) => {
                    conflicts.push(c.path);
                }
            }
        }

        if !conflicts.is_empty() {
            return Err(ForgeError::Conflict(format!(
                "Cannot merge due to conflicts in: {}",
                conflicts.join(", ")
            )));
        }

        // Construct merged tree
        let root_tree_oid = self.write_tree_recursive(&store, &merged_map)?;
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

        let commit = Commit::new(
            root_tree_oid,
            vec![base_oid, head_oid],
            author_name.to_string(),
            author_email.to_string(),
            author_name.to_string(),
            author_email.to_string(),
            now,
            commit_message.to_string(),
        );

        let commit_payload = serialize_commit(&commit);
        let merge_commit_oid = store
            .write_object(ObjectType::Commit, &commit_payload)
            .map_err(|e| ForgeError::Internal(format!("Failed to write merge commit: {}", e)))?;

        // Update target branch ref
        let spine_dir = self.repo_spine_dir(native_repo_id)?;
        let branch_ref_file = spine_dir.join("refs").join("heads").join(base_branch);
        if let Some(parent) = branch_ref_file.parent() {
            fs::create_dir_all(parent).map_err(|e| ForgeError::Internal(e.to_string()))?;
        }
        fs::write(&branch_ref_file, format!("{}\n", merge_commit_oid.to_hex()))
            .map_err(|e| ForgeError::Internal(format!("Failed to update branch ref: {}", e)))?;

        Ok(MergeResultDto {
            success: true,
            merge_commit_id: Some(merge_commit_oid.to_hex()),
            message: format!("Successfully merged {} into {}", head_branch, base_branch),
        })
    }
}
