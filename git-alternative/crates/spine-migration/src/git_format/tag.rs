use super::oid::GitOid;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TagFormatError {
    #[error("Missing object header in tag object")]
    MissingObject,
    #[error("Missing type header in tag object")]
    MissingType,
    #[error("Missing tag name header in tag object")]
    MissingTagName,
    #[error("Missing tagger header in tag object")]
    MissingTagger,
    #[error("Invalid Git OID in tag header: {0}")]
    InvalidOid(String),
    #[error("Invalid UTF-8 in tag payload")]
    InvalidUtf8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitAnnotatedTag {
    pub object: GitOid,
    pub target_type: String,
    pub tag_name: String,
    pub tagger: String,
    pub message: String,
}

/// Parses a plaintext Git annotated tag payload.
pub fn parse_git_tag(payload: &[u8]) -> Result<GitAnnotatedTag, TagFormatError> {
    let s = std::str::from_utf8(payload).map_err(|_| TagFormatError::InvalidUtf8)?;

    let mut object = None;
    let mut target_type = None;
    let mut tag_name = None;
    let mut tagger = None;
    let mut body_start_idx = 0;

    for line in s.lines() {
        if line.is_empty() {
            let marker = "\n\n";
            if let Some(pos) = s.find(marker) {
                body_start_idx = pos + 2;
            } else if let Some(pos) = s.find("\r\n\r\n") {
                body_start_idx = pos + 4;
            }
            break;
        }

        if let Some(o) = line.strip_prefix("object ") {
            object = Some(GitOid::from_hex(o).map_err(|_| TagFormatError::InvalidOid(o.to_string()))?);
        } else if let Some(t) = line.strip_prefix("type ") {
            target_type = Some(t.to_string());
        } else if let Some(n) = line.strip_prefix("tag ") {
            tag_name = Some(n.to_string());
        } else if let Some(tr) = line.strip_prefix("tagger ") {
            tagger = Some(tr.to_string());
        }
    }

    let object = object.ok_or(TagFormatError::MissingObject)?;
    let target_type = target_type.ok_or(TagFormatError::MissingType)?;
    let tag_name = tag_name.ok_or(TagFormatError::MissingTagName)?;
    let tagger = tagger.unwrap_or_else(|| "Unknown <unknown@spine.local> 0 +0000".to_string());
    let message = if body_start_idx < s.len() {
        s[body_start_idx..].to_string()
    } else {
        String::new()
    };

    Ok(GitAnnotatedTag {
        object,
        target_type,
        tag_name,
        tagger,
        message,
    })
}

/// Serializes a GitAnnotatedTag into canonical plaintext Git tag format.
pub fn serialize_git_tag(tag: &GitAnnotatedTag) -> Vec<u8> {
    let mut s = String::new();
    s.push_str(&format!("object {}\n", tag.object));
    s.push_str(&format!("type {}\n", tag.target_type));
    s.push_str(&format!("tag {}\n", tag.tag_name));
    s.push_str(&format!("tagger {}\n", tag.tagger));
    s.push('\n');
    s.push_str(&tag.message);

    s.into_bytes()
}
