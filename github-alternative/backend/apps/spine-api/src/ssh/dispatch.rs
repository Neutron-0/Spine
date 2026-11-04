use thiserror::Error;

#[derive(Debug, PartialEq, Eq)]
pub enum GitSshCommand {
    UploadPack {
        owner: String,
        repo: String,
    },
    ReceivePack {
        owner: String,
        repo: String,
    },
    LfsAuthenticate {
        owner: String,
        repo: String,
        operation: String, // "upload" | "download"
    },
}

impl GitSshCommand {
    pub fn parse(raw: &str) -> Result<Self, DispatchError> {
        parse_git_ssh_command(raw)
    }
}

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error("Empty or invalid command")]
    InvalidCommand,
    #[error("Command not permitted in restricted Git SSH shell: {0}")]
    ForbiddenCommand(String),
    #[error("Invalid repository path: {0}")]
    InvalidPath(String),
    #[error("Invalid LFS operation: {0}")]
    InvalidLfsOperation(String),
}

/// Strictly parses and validates restricted Git/SSH commands.
/// Prohibits shell metacharacters, traversal, and unapproved commands.
pub fn parse_git_ssh_command(raw: &str) -> Result<GitSshCommand, DispatchError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(DispatchError::InvalidCommand);
    }

    // Prohibit dangerous shell metacharacters
    for ch in ['\n', '\r', ';', '&', '|', '`', '$', '<', '>', '(', ')'] {
        if trimmed.contains(ch) {
            return Err(DispatchError::ForbiddenCommand(
                "Command contains forbidden shell metacharacters".to_string(),
            ));
        }
    }

    // Split command into tokens respecting optional single/double quoting around arguments
    let tokens = tokenize_command(trimmed)?;
    if tokens.is_empty() {
        return Err(DispatchError::InvalidCommand);
    }

    let verb = tokens[0].as_str();

    match verb {
        "git-upload-pack" | "git upload-pack" => {
            if tokens.len() != 2 {
                return Err(DispatchError::InvalidCommand);
            }
            let (owner, repo) = parse_repo_path(&tokens[1])?;
            Ok(GitSshCommand::UploadPack { owner, repo })
        }
        "git-receive-pack" | "git receive-pack" => {
            if tokens.len() != 2 {
                return Err(DispatchError::InvalidCommand);
            }
            let (owner, repo) = parse_repo_path(&tokens[1])?;
            Ok(GitSshCommand::ReceivePack { owner, repo })
        }
        "git-lfs-authenticate" => {
            if tokens.len() != 3 {
                return Err(DispatchError::InvalidCommand);
            }
            let (owner, repo) = parse_repo_path(&tokens[1])?;
            let op = tokens[2].to_lowercase();
            if op != "upload" && op != "download" {
                return Err(DispatchError::InvalidLfsOperation(op));
            }
            Ok(GitSshCommand::LfsAuthenticate {
                owner,
                repo,
                operation: op,
            })
        }
        other => Err(DispatchError::ForbiddenCommand(other.to_string())),
    }
}

/// Tokenizes a simple command string, respecting single or double quotes.
fn tokenize_command(s: &str) -> Result<Vec<String>, DispatchError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quote: Option<char> = None;
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\'' | '"' => {
                if let Some(q) = in_quote {
                    if q == ch {
                        in_quote = None;
                    } else {
                        current.push(ch);
                    }
                } else {
                    in_quote = Some(ch);
                }
            }
            ' ' | '\t' => {
                if in_quote.is_some() {
                    current.push(ch);
                } else if !current.is_empty() {
                    tokens.push(current);
                    current = String::new();
                }
            }
            _ => {
                current.push(ch);
            }
        }
    }

    if in_quote.is_some() {
        return Err(DispatchError::InvalidCommand);
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    // Handle "git upload-pack" and "git receive-pack" 2-word commands
    if tokens.len() >= 2 && tokens[0] == "git" {
        if tokens[1] == "upload-pack" || tokens[1] == "receive-pack" {
            let combined = format!("git-{}", tokens[1]);
            let mut new_tokens = vec![combined];
            new_tokens.extend(tokens.into_iter().skip(2));
            return Ok(new_tokens);
        }
    }

    Ok(tokens)
}

/// Parses an owner/repo or /owner/repo.git path into (owner, repo_name)
fn parse_repo_path(raw_path: &str) -> Result<(String, String), DispatchError> {
    let clean = raw_path
        .trim()
        .trim_matches('\'')
        .trim_matches('"')
        .trim_start_matches('/');

    let without_git = clean.strip_suffix(".git").unwrap_or(clean);

    // Reject path traversal
    if without_git.contains("..") || without_git.contains('\\') {
        return Err(DispatchError::InvalidPath(raw_path.to_string()));
    }

    let parts: Vec<&str> = without_git.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() != 2 {
        return Err(DispatchError::InvalidPath(raw_path.to_string()));
    }

    let owner = parts[0].to_string();
    let repo = parts[1].to_string();

    if owner.is_empty() || repo.is_empty() {
        return Err(DispatchError::InvalidPath(raw_path.to_string()));
    }

    Ok((owner, repo))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_commands() {
        assert_eq!(
            parse_git_ssh_command("git-upload-pack 'owner/repo.git'").unwrap(),
            GitSshCommand::UploadPack {
                owner: "owner".into(),
                repo: "repo".into()
            }
        );

        assert_eq!(
            parse_git_ssh_command("git upload-pack /alice/myproject").unwrap(),
            GitSshCommand::UploadPack {
                owner: "alice".into(),
                repo: "myproject".into()
            }
        );

        assert_eq!(
            parse_git_ssh_command("git-receive-pack 'bob/demo.git'").unwrap(),
            GitSshCommand::ReceivePack {
                owner: "bob".into(),
                repo: "demo".into()
            }
        );

        assert_eq!(
            parse_git_ssh_command("git-lfs-authenticate 'org/data.git' download").unwrap(),
            GitSshCommand::LfsAuthenticate {
                owner: "org".into(),
                repo: "data".into(),
                operation: "download".into()
            }
        );

        assert!(parse_git_ssh_command("bash -i").is_err());
        assert!(parse_git_ssh_command("git-upload-pack 'repo'; rm -rf /").is_err());
        assert!(parse_git_ssh_command("git-upload-pack '../etc/passwd'").is_err());
    }
}
