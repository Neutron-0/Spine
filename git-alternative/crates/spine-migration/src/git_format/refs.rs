use super::oid::GitOid;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RefError {
    #[error("I/O error reading/writing refs: {0}")]
    Io(#[from] io::Error),
    #[error("Invalid reference format: {0}")]
    InvalidRef(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHead {
    Symbolic(String), // e.g. "refs/heads/main"
    Detached(GitOid),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GitRefMap {
    pub head: Option<GitHead>,
    pub branches: BTreeMap<String, GitOid>, // branch name (e.g. "main") -> oid
    pub tags: BTreeMap<String, GitOid>,     // tag name -> oid
    pub peeled_tags: BTreeMap<String, GitOid>, // tag name -> peeled commit oid
    pub other_refs: BTreeMap<String, GitOid>, // arbitrary ref (e.g. "refs/notes/commits") -> oid
}

/// Reads all references from a `.git` or bare repository directory.
pub fn read_git_refs<P: AsRef<Path>>(git_dir: P) -> Result<GitRefMap, RefError> {
    let git_dir = git_dir.as_ref();
    let mut ref_map = GitRefMap::default();

    // 1. Read HEAD
    let head_path = git_dir.join("HEAD");
    if head_path.exists() {
        let content = fs::read_to_string(&head_path)?;
        let trimmed = content.trim();
        if let Some(target) = trimmed.strip_prefix("ref: ") {
            ref_map.head = Some(GitHead::Symbolic(target.trim().to_string()));
        } else if let Ok(oid) = GitOid::from_hex(trimmed) {
            ref_map.head = Some(GitHead::Detached(oid));
        }
    }

    // 2. Read packed-refs if present
    let packed_refs_path = git_dir.join("packed-refs");
    if packed_refs_path.exists() {
        let content = fs::read_to_string(&packed_refs_path)?;
        let mut last_tag: Option<String> = None;

        for line in content.lines() {
            let line = line.trim();
            if line.starts_with('#') || line.is_empty() {
                continue;
            }

            if let Some(peeled_hex) = line.strip_prefix('^') {
                if let (Some(tag), Ok(peeled_oid)) = (last_tag.take(), GitOid::from_hex(peeled_hex)) {
                    ref_map.peeled_tags.insert(tag, peeled_oid);
                }
                continue;
            }

            let mut parts = line.splitn(2, ' ');
            let oid_str = parts.next().unwrap_or("");
            let ref_name = parts.next().unwrap_or("");

            if let Ok(oid) = GitOid::from_hex(oid_str) {
                if let Some(branch) = ref_name.strip_prefix("refs/heads/") {
                    ref_map.branches.insert(branch.to_string(), oid);
                    last_tag = None;
                } else if let Some(tag) = ref_name.strip_prefix("refs/tags/") {
                    ref_map.tags.insert(tag.to_string(), oid);
                    last_tag = Some(tag.to_string());
                } else {
                    ref_map.other_refs.insert(ref_name.to_string(), oid);
                    last_tag = None;
                }
            }
        }
    }

    // 3. Read loose refs from refs/heads/ and refs/tags/
    let heads_dir = git_dir.join("refs").join("heads");
    if heads_dir.exists() {
        read_loose_refs_recursive(&heads_dir, "", &mut ref_map.branches)?;
    }

    let tags_dir = git_dir.join("refs").join("tags");
    if tags_dir.exists() {
        read_loose_refs_recursive(&tags_dir, "", &mut ref_map.tags)?;
    }

    Ok(ref_map)
}

fn read_loose_refs_recursive(
    dir: &Path,
    prefix: &str,
    out: &mut BTreeMap<String, GitOid>,
) -> Result<(), RefError> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let full_name = if prefix.is_empty() {
            name
        } else {
            format!("{}/{}", prefix, name)
        };

        if path.is_dir() {
            read_loose_refs_recursive(&path, &full_name, out)?;
        } else if path.is_file() {
            let content = fs::read_to_string(&path)?;
            let trimmed = content.trim();
            if let Ok(oid) = GitOid::from_hex(trimmed) {
                out.insert(full_name, oid);
            }
        }
    }
    Ok(())
}

/// Writes all references into a `.git` or bare directory structure.
pub fn write_git_refs<P: AsRef<Path>>(git_dir: P, ref_map: &GitRefMap) -> Result<(), RefError> {
    let git_dir = git_dir.as_ref();

    // 1. Write HEAD
    if let Some(ref head) = ref_map.head {
        let head_path = git_dir.join("HEAD");
        let content = match head {
            GitHead::Symbolic(target) => format!("ref: {}\n", target),
            GitHead::Detached(oid) => format!("{}\n", oid),
        };
        fs::write(head_path, content)?;
    }

    // 2. Write loose branches
    let heads_dir = git_dir.join("refs").join("heads");
    fs::create_dir_all(&heads_dir)?;
    for (branch, oid) in &ref_map.branches {
        let branch_path = heads_dir.join(branch);
        if let Some(parent) = branch_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(branch_path, format!("{}\n", oid))?;
    }

    // 3. Write loose tags
    let tags_dir = git_dir.join("refs").join("tags");
    fs::create_dir_all(&tags_dir)?;
    for (tag, oid) in &ref_map.tags {
        let tag_path = tags_dir.join(tag);
        if let Some(parent) = tag_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(tag_path, format!("{}\n", oid))?;
    }

    // 4. Optionally write packed-refs
    let mut packed = String::from("# pack-refs with: peeled fully-peeled sorted \n");
    for (tag, oid) in &ref_map.tags {
        packed.push_str(&format!("{} refs/tags/{}\n", oid, tag));
        if let Some(peeled) = ref_map.peeled_tags.get(tag) {
            packed.push_str(&format!("^{}\n", peeled));
        }
    }
    for (branch, oid) in &ref_map.branches {
        packed.push_str(&format!("{} refs/heads/{}\n", oid, branch));
    }
    fs::write(git_dir.join("packed-refs"), packed)?;

    Ok(())
}
