//! # spine-diff
//!
//! Myers diff and longest common subsequence algorithms for line-level file comparison
//! and unified diff generation.

use spine_core::{FileMode, ObjectId};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffOp {
    Context(String),
    Addition(String),
    Deletion(String),
}

/// Represents a change to a single tracked path between two tree snapshots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeChange {
    Added {
        path: String,
        object_id: ObjectId,
    },
    Deleted {
        path: String,
        object_id: ObjectId,
    },
    Modified {
        path: String,
        old_id: ObjectId,
        new_id: ObjectId,
    },
}

impl TreeChange {
    pub fn path(&self) -> &str {
        match self {
            Self::Added { path, .. } => path,
            Self::Deleted { path, .. } => path,
            Self::Modified { path, .. } => path,
        }
    }
}

/// Computes differences between two flattened tree structures.
pub fn compare_trees(
    tree_a: &BTreeMap<String, (FileMode, ObjectId)>,
    tree_b: &BTreeMap<String, (FileMode, ObjectId)>,
) -> Vec<TreeChange> {
    let mut changes = Vec::new();

    // Check additions and modifications
    for (path, (_mode_b, oid_b)) in tree_b {
        match tree_a.get(path) {
            Some((_mode_a, oid_a)) => {
                if oid_a != oid_b {
                    changes.push(TreeChange::Modified {
                        path: path.clone(),
                        old_id: *oid_a,
                        new_id: *oid_b,
                    });
                }
            }
            None => {
                changes.push(TreeChange::Added {
                    path: path.clone(),
                    object_id: *oid_b,
                });
            }
        }
    }

    // Check deletions
    for (path, (_mode_a, oid_a)) in tree_a {
        if !tree_b.contains_key(path) {
            changes.push(TreeChange::Deleted {
                path: path.clone(),
                object_id: *oid_a,
            });
        }
    }

    changes.sort_by(|a, b| a.path().cmp(b.path()));
    changes
}

/// Statistics summary for a diff.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DiffSummary {
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
    pub file_stats: Vec<(String, usize, usize, bool)>, // (path, adds, dels, is_binary)
}

/// Formats diff summary into standard `--stat` display table.
pub fn format_diff_stat(summary: &DiffSummary) -> String {
    use std::fmt::Write;
    let mut out = String::new();

    if summary.file_stats.is_empty() {
        return out;
    }

    let max_path_len = summary
        .file_stats
        .iter()
        .map(|(p, _, _, _)| p.len())
        .max()
        .unwrap_or(0);

    for (path, adds, dels, is_bin) in &summary.file_stats {
        if *is_bin {
            let _ = writeln!(&mut out, " {:width$} | Bin", path, width = max_path_len);
        } else {
            let total = adds + dels;
            let pluses = "+".repeat((*adds).min(30));
            let minuses = "-".repeat((*dels).min(30));
            let _ = writeln!(
                &mut out,
                " {:width$} | {:>3} {}{}",
                path,
                total,
                pluses,
                minuses,
                width = max_path_len
            );
        }
    }

    let _ = writeln!(
        &mut out,
        " {} files changed, {} insertions(+), {} deletions(-)",
        summary.files_changed, summary.insertions, summary.deletions
    );

    out
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

    #[test]
    fn test_compare_trees_and_stats() {
        let mut a = BTreeMap::new();
        a.insert(
            "file1.txt".into(),
            (FileMode::Regular, ObjectId::from_bytes([1u8; 32])),
        );
        a.insert(
            "file2.txt".into(),
            (FileMode::Regular, ObjectId::from_bytes([2u8; 32])),
        );

        let mut b = BTreeMap::new();
        b.insert(
            "file1.txt".into(),
            (FileMode::Regular, ObjectId::from_bytes([1u8; 32])),
        ); // unchanged
        b.insert(
            "file2.txt".into(),
            (FileMode::Regular, ObjectId::from_bytes([3u8; 32])),
        ); // modified
        b.insert(
            "file3.txt".into(),
            (FileMode::Regular, ObjectId::from_bytes([4u8; 32])),
        ); // added

        let changes = compare_trees(&a, &b);
        assert_eq!(changes.len(), 2);
        assert!(matches!(changes[0], TreeChange::Modified { .. }));
        assert!(matches!(changes[1], TreeChange::Added { .. }));

        let summary = DiffSummary {
            files_changed: 2,
            insertions: 5,
            deletions: 1,
            file_stats: vec![
                ("file2.txt".into(), 2, 1, false),
                ("file3.txt".into(), 3, 0, false),
            ],
        };
        let stat_str = format_diff_stat(&summary);
        assert!(stat_str.contains("file2.txt |   3 ++-"));
        assert!(stat_str.contains("2 files changed, 5 insertions(+), 1 deletions(-)"));
    }
}
