//! Spine repository URL parser and validator.

use std::fmt;
use thiserror::Error;

pub const DEFAULT_SPINE_PORT: u16 = 4434;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum UrlError {
    #[error("Invalid URL scheme: expected 'spine://', got '{0}'")]
    InvalidScheme(String),
    #[error("Missing host in repository URL: '{0}'")]
    MissingHost(String),
    #[error("Missing repository path in URL: '{0}'")]
    MissingPath(String),
    #[error("Invalid port in repository URL: '{0}'")]
    InvalidPort(String),
}

/// Parsed Spine repository URL: `spine://[user@]host[:port]/path`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpineUrl {
    pub user: Option<String>,
    pub host: String,
    pub port: u16,
    pub path: String,
}

impl SpineUrl {
    pub fn parse(url_str: &str) -> Result<Self, UrlError> {
        let trimmed = url_str.trim();
        let rest = if let Some(stripped) = trimmed.strip_prefix("spine://") {
            stripped
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
            user,
            host,
            port,
            path,
        })
    }

    pub fn to_address_string(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

impl fmt::Display for SpineUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "spine://")?;
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
        assert_eq!(url.user, None);
        assert_eq!(url.host, "127.0.0.1");
        assert_eq!(url.port, 4434);
        assert_eq!(url.path, "repos/project");
        assert_eq!(url.to_address_string(), "127.0.0.1:4434");

        let url_user = SpineUrl::parse("spine://alice@example.com/team/app").unwrap();
        assert_eq!(url_user.user.as_deref(), Some("alice"));
        assert_eq!(url_user.host, "example.com");
        assert_eq!(url_user.port, DEFAULT_SPINE_PORT);
        assert_eq!(url_user.path, "team/app");

        assert!(SpineUrl::parse("http://example.com/repo").is_err());
        assert!(SpineUrl::parse("spine://example.com").is_err());
    }
}
