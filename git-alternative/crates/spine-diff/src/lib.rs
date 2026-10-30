//! # spine-diff
//!
//! Myers diff and longest common subsequence algorithms for line-level file comparison
//! and unified diff generation.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffOp {
    Context(String),
    Addition(String),
    Deletion(String),
}

/// Computes line-by-line diff between two text documents using LCS algorithm.
pub fn compute_line_diff(old_text: &str, new_text: &str) -> Vec<DiffOp> {
    let old_lines: Vec<&str> = old_text.lines().collect();
    let new_lines: Vec<&str> = new_text.lines().collect();

    let n = old_lines.len();
    let m = new_lines.len();

    // Standard LCS table
    let mut dp = vec![vec![0usize; m + 1]; n + 1];

    for i in 0..n {
        for j in 0..m {
            if old_lines[i] == new_lines[j] {
                dp[i + 1][j + 1] = dp[i][j] + 1;
            } else {
                dp[i + 1][j + 1] = dp[i + 1][j].max(dp[i][j + 1]);
            }
        }
    }

    // Backtrack to build diff ops
    let mut i = n;
    let mut j = m;
    let mut ops = Vec::new();

    while i > 0 || j > 0 {
        if i > 0 && j > 0 && old_lines[i - 1] == new_lines[j - 1] {
            ops.push(DiffOp::Context(old_lines[i - 1].to_string()));
            i -= 1;
            j -= 1;
        } else if j > 0 && (i == 0 || dp[i][j - 1] >= dp[i - 1][j]) {
            ops.push(DiffOp::Addition(new_lines[j - 1].to_string()));
            j -= 1;
        } else if i > 0 && (j == 0 || dp[i][j - 1] < dp[i - 1][j]) {
            ops.push(DiffOp::Deletion(old_lines[i - 1].to_string()));
            i -= 1;
        }
    }

    ops.reverse();
    ops
}

/// Formats line diff into a standard unified diff string.
pub fn format_unified_diff(path: &str, old_text: &str, new_text: &str) -> String {
    let ops = compute_line_diff(old_text, new_text);
    let mut out = String::new();

    let has_changes = ops
        .iter()
        .any(|op| matches!(op, DiffOp::Addition(_) | DiffOp::Deletion(_)));
    if !has_changes {
        return out;
    }

    use std::fmt::Write;
    let _ = writeln!(&mut out, "--- a/{}", path);
    let _ = writeln!(&mut out, "+++ b/{}", path);

    for op in ops {
        match op {
            DiffOp::Context(line) => {
                let _ = writeln!(&mut out, " {}", line);
            }
            DiffOp::Addition(line) => {
                let _ = writeln!(&mut out, "+{}", line);
            }
            DiffOp::Deletion(line) => {
                let _ = writeln!(&mut out, "-{}", line);
            }
        }
    }

    out
}

/// Checks if byte slice represents binary content by scanning for null bytes.
pub fn is_binary(bytes: &[u8]) -> bool {
    let check_len = bytes.len().min(8000);
    bytes[..check_len].contains(&0x00)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diff_operations() {
        let old = "line 1\nline 2\nline 3";
        let new = "line 1\nline 2 modified\nline 3\nline 4";

        let ops = compute_line_diff(old, new);
        assert!(ops.contains(&DiffOp::Context("line 1".into())));
        assert!(ops.contains(&DiffOp::Deletion("line 2".into())));
        assert!(ops.contains(&DiffOp::Addition("line 2 modified".into())));
        assert!(ops.contains(&DiffOp::Addition("line 4".into())));
    }
}
