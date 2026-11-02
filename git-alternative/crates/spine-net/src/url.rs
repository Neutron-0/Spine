//! Spine repository URL parser and validator.

use std::fmt;
use thiserror::Error;

pub const DEFAULT_SPINE_PORT: u16 = 4434;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum UrlError {
    #[error("Invalid URL scheme: expected 'spine://' or 'spinelocal://', got '{0}'")]
    InvalidScheme(String),
    #[error("Missing host in repository URL: '{0}'")]
    MissingHost(String),
    #[error("Missing repository path in URL: '{0}'")]
    MissingPath(String),
    #[error("Invalid port in repository URL: '{0}'")]
    InvalidPort(String),
}

/// Parsed Spine repository URL: `spine://[user@]host[:port]/path` or `spinelocal://[user@]host[:port]/path`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpineUrl {
    pub scheme: String,
    pub user: Option<String>,
    pub host: String,
    pub port: u16,
    pub path: String,
}

impl SpineUrl {
    pub fn parse(url_str: &str) -> Result<Self, UrlError> {
        let trimmed = url_str.trim();
        let (scheme, rest) = if let Some(stripped) = trimmed.strip_prefix("spinelocal://") {
            ("spinelocal".to_string(), stripped)
        } else if let Some(stripped) = trimmed.strip_prefix("spine://") {
            ("spine".to_string(), stripped)
        } else {
            return Err(UrlError::InvalidScheme(trimmed.to_string()));
        };

        let (authority, path_str) = match rest.find('/') {
            Some(idx) => (&rest[..idx], &rest[idx + 1..]),
            None => (rest, ""),
        };

        if path_str.is_empty() {
            return Err(UrlError::MissingPath(url_str.to_string()));
        }

        let (user, host_port) = match authority.find('@') {
            Some(idx) => (Some(authority[..idx].to_string()), &authority[idx + 1..]),
            None => (None, authority),
        };

        let (host, port) = match host_port.rfind(':') {
            Some(idx) => {
                let h = &host_port[..idx];
                let p_str = &host_port[idx + 1..];
                let p = p_str
                    .parse::<u16>()
                    .map_err(|_| UrlError::InvalidPort(p_str.to_string()))?;
                (h.to_string(), p)
            }
            None => (host_port.to_string(), DEFAULT_SPINE_PORT),
        };

        if host.is_empty() {
            return Err(UrlError::MissingHost(url_str.to_string()));
        }

        // Clean path to remove leading/trailing slashes
        let path = path_str.trim_matches('/').to_string();

        Ok(Self {
            scheme,
            user,
            host,
            port,
            path,
        })
    }

    /// Returns true if this URL specifies TLS encryption (`spine://`).
    pub fn requires_tls(&self) -> bool {
        self.scheme == "spine"
    }

    /// Returns true if the target host is loopback (127.0.0.1, localhost, or ::1).
    pub fn is_loopback(&self) -> bool {
        self.host == "127.0.0.1" || self.host == "localhost" || self.host == "::1"
    }

    pub fn to_address_string(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

impl fmt::Display for SpineUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}://", self.scheme)?;
        if let Some(user) = &self.user {
            write!(f, "{}@", user)?;
        }
        write!(f, "{}:{}/{}", self.host, self.port, self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_spine_url() {
        let url = SpineUrl::parse("spine://127.0.0.1:4434/repos/project").unwrap();
        assert_eq!(url.scheme, "spine");
        assert_eq!(url.host, "127.0.0.1");
        assert_eq!(url.port, 4434);
        assert_eq!(url.path, "repos/project");
        assert!(url.requires_tls());
        assert!(url.is_loopback());

        let local_url = SpineUrl::parse("spinelocal://alice@127.0.0.1:4435/local-repo").unwrap();
        assert_eq!(local_url.scheme, "spinelocal");
        assert_eq!(local_url.user.as_deref(), Some("alice"));
        assert!(!local_url.requires_tls());
        assert!(local_url.is_loopback());
    }
}
