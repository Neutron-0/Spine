use super::oid::GitOid;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CommitFormatError {
    #[error("Missing tree header in commit object")]
    MissingTree,
    #[error("Invalid Git OID in commit header: {0}")]
    InvalidOid(String),
    #[error("Missing author header in commit object")]
    MissingAuthor,
    #[error("Missing committer header in commit object")]
    MissingCommitter,
    #[error("Malformed commit payload encoding")]
    InvalidUtf8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommit {
    pub tree: GitOid,
    pub parents: Vec<GitOid>,
    pub author: String,
    pub committer: String,
    pub gpgsig: Option<String>,
    pub message: String,
}

impl GitCommit {
    /// Extracts author name, email, and timestamp if available in `Name <email> timestamp tz` format.
    pub fn parse_author(&self) -> (String, String, i64) {
        parse_identity_line(&self.author)
    }

    /// Extracts committer name, email, and timestamp if available.
    pub fn parse_committer(&self) -> (String, String, i64) {
        parse_identity_line(&self.committer)
    }
}

pub fn parse_identity_line(line: &str) -> (String, String, i64) {
    let mut name = "Unknown";
    let mut email = "unknown@spine.local";
    let mut timestamp = 0i64;

    if let Some(open) = line.find('<') {
        name = line[..open].trim();
        if let Some(close) = line[open..].find('>') {
            email = &line[open + 1..open + close];
            let rest = line[open + close + 1..].trim();
            if let Some(space) = rest.find(' ') {
                if let Ok(ts) = rest[..space].parse::<i64>() {
                    timestamp = ts;
                }
            } else if let Ok(ts) = rest.parse::<i64>() {
                timestamp = ts;
            }
        }
    }

    (name.to_string(), email.to_string(), timestamp)
}

/// Parses a plaintext Git commit object payload.
pub fn parse_git_commit(payload: &[u8]) -> Result<GitCommit, CommitFormatError> {
    let s = std::str::from_utf8(payload).map_err(|_| CommitFormatError::InvalidUtf8)?;

    let mut tree = None;
    let mut parents = Vec::new();
    let mut author = None;
    let mut committer = None;
    let mut gpgsig = None;
    let mut in_gpgsig = false;
    let mut gpgsig_buf = String::new();

    let mut body_start_idx = 0;

    for line in s.lines() {
        if in_gpgsig {
            if line.starts_with(' ') {
                gpgsig_buf.push('\n');
                gpgsig_buf.push_str(line.strip_prefix(' ').unwrap_or(line));
                continue;
            } else {
                in_gpgsig = false;
                gpgsig = Some(gpgsig_buf.clone());
            }
        }

        if line.is_empty() {
            // Empty line marks end of headers and start of commit message
            let marker = "\n\n";
            if let Some(pos) = s.find(marker) {
                body_start_idx = pos + 2;
            } else if let Some(pos) = s.find("\r\n\r\n") {
                body_start_idx = pos + 4;
            }
            break;
        }

        if let Some(t) = line.strip_prefix("tree ") {
            tree = Some(
                GitOid::from_hex(t).map_err(|_| CommitFormatError::InvalidOid(t.to_string()))?,
            );
        } else if let Some(p) = line.strip_prefix("parent ") {
            parents.push(
                GitOid::from_hex(p).map_err(|_| CommitFormatError::InvalidOid(p.to_string()))?,
            );
        } else if let Some(a) = line.strip_prefix("author ") {
            author = Some(a.to_string());
        } else if let Some(c) = line.strip_prefix("committer ") {
            committer = Some(c.to_string());
        } else if let Some(sig) = line.strip_prefix("gpgsig ") {
            in_gpgsig = true;
            gpgsig_buf.clear();
            gpgsig_buf.push_str(sig);
        }
    }

    if in_gpgsig {
        gpgsig = Some(gpgsig_buf);
    }

    let tree = tree.ok_or(CommitFormatError::MissingTree)?;
    let author = author.ok_or(CommitFormatError::MissingAuthor)?;
    let committer = committer.ok_or(CommitFormatError::MissingCommitter)?;
    let message = if body_start_idx < s.len() {
        s[body_start_idx..].to_string()
    } else {
        String::new()
    };

    Ok(GitCommit {
        tree,
        parents,
        author,
        committer,
        gpgsig,
        message,
    })
}

/// Serializes a GitCommit into canonical plaintext Git commit format.
pub fn serialize_git_commit(commit: &GitCommit) -> Vec<u8> {
    let mut s = String::new();
    s.push_str(&format!("tree {}\n", commit.tree));
    for parent in &commit.parents {
        s.push_str(&format!("parent {}\n", parent));
    }
    s.push_str(&format!("author {}\n", commit.author));
    s.push_str(&format!("committer {}\n", commit.committer));
    if let Some(ref sig) = commit.gpgsig {
        s.push_str("gpgsig");
        for line in sig.lines() {
            s.push(' ');
            s.push_str(line);
            s.push('\n');
        }
    }
    s.push('\n');
    s.push_str(&commit.message);

    s.into_bytes()
}
