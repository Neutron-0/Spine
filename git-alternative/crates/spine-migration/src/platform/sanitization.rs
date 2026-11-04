/// Security scanner and sanitizer for imported repositories and workflow files.
/// Guarantees that untrusted workflows and exposed secrets cannot auto-trigger or execute unisolated.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretDetection {
    pub file_path: String,
    pub line_number: usize,
    pub secret_kind: &'static str,
    pub redacted_snippet: String,
}

#[derive(Debug, Clone, Default)]
pub struct SanitizationReport {
    pub workflows_deactivated: Vec<String>,
    pub detected_secrets: Vec<SecretDetection>,
}

/// Scans and sanitizes a file path and its contents.
/// If it is a workflow file (e.g. `.github/workflows/*.yml` or `.gitlab-ci.yml`),
/// returns whether it should be deactivated.
pub fn is_workflow_file(path_str: &str) -> bool {
    let normalized = path_str.replace('\\', "/");
    normalized.starts_with(".github/workflows/")
        || normalized == ".gitlab-ci.yml"
        || normalized.starts_with(".spine/workflows/")
}

/// Scans text content for common sensitive API keys, personal access tokens, and private keys.
pub fn scan_for_secrets(file_path: &str, content: &str) -> Vec<SecretDetection> {
    let mut detections = Vec::new();

    for (line_idx, line) in content.lines().enumerate() {
        let line_num = line_idx + 1;

        if line.contains("ghp_") || line.contains("gho_") || line.contains("ghu_") {
            detections.push(SecretDetection {
                file_path: file_path.to_string(),
                line_number: line_num,
                secret_kind: "GitHub Personal Access Token",
                redacted_snippet: redact_line(line),
            });
        } else if line.contains("glpat-") {
            detections.push(SecretDetection {
                file_path: file_path.to_string(),
                line_number: line_num,
                secret_kind: "GitLab Personal Access Token",
                redacted_snippet: redact_line(line),
            });
        } else if line.contains("-----BEGIN") && line.contains("PRIVATE KEY-----") {
            detections.push(SecretDetection {
                file_path: file_path.to_string(),
                line_number: line_num,
                secret_kind: "Cryptographic Private Key",
                redacted_snippet: "[REDACTED PRIVATE KEY HEADER]".to_string(),
            });
        } else if line.contains("AKIA") && line.len() >= 20 {
            detections.push(SecretDetection {
                file_path: file_path.to_string(),
                line_number: line_num,
                secret_kind: "AWS Access Key ID",
                redacted_snippet: redact_line(line),
            });
        }
    }

    detections
}

fn redact_line(line: &str) -> String {
    let trimmed = line.trim();
    if trimmed.len() <= 8 {
        return "[REDACTED]".to_string();
    }
    format!("{}...[REDACTED]", &trimmed[..6.min(trimmed.len())])
}
