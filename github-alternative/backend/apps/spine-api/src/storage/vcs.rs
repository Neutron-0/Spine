use crate::core::error::ForgeError;
use serde::Serialize;
use spine_core::{FileMode, HashAlgorithm, ObjectId, ObjectType};
use spine_format::{deserialize_commit, deserialize_tree};
use spine_object_store::{CompositeObjectStore, ObjectStore};
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
}
