use spine_merge::{merge_3way_text, TextMergeResult};

#[test]
fn test_offset_insertion_and_modification_clean_merge() {
    let base = "line 1\nline 2\nline 3\nline 4\nline 5\n";
    let ours = "line 1\ninserted in ours\nline 2\nline 3\nline 4\nline 5\n";
    let theirs = "line 1\nline 2\nline 3\nline 4\nline 5 modified in theirs\n";

    let result = merge_3way_text(base, ours, theirs);

    let expected = "line 1\ninserted in ours\nline 2\nline 3\nline 4\nline 5 modified in theirs\n";

    match result {
        TextMergeResult::Clean(merged) => {
            assert_eq!(merged, expected);
        }
        TextMergeResult::Conflict(conflict_text) => {
            panic!(
                "Expected clean merge for disjoint changes, but got conflict:\n{}",
                conflict_text
            );
        }
    }
}

#[test]
fn test_both_insert_different_locations() {
    let base = "alpha\nbeta\ngamma\n";
    let ours = "alpha\ninserted ours\nbeta\ngamma\n";
    let theirs = "alpha\nbeta\ngamma\ninserted theirs\n";

    let result = merge_3way_text(base, ours, theirs);
    let expected = "alpha\ninserted ours\nbeta\ngamma\ninserted theirs\n";

    match result {
        TextMergeResult::Clean(merged) => {
            assert_eq!(merged, expected);
        }
        TextMergeResult::Conflict(conflict_text) => {
            panic!("Expected clean merge, got conflict:\n{}", conflict_text);
        }
    }
}

#[test]
fn test_adjacent_line_independent_edits_clean_merge() {
    let base = "line 1\nline 2\nline 3\n";
    let ours = "line 1 modified by ours\nline 2\nline 3\n";
    let theirs = "line 1\nline 2 modified by theirs\nline 3\n";

    let result = merge_3way_text(base, ours, theirs);
    let expected = "line 1 modified by ours\nline 2 modified by theirs\nline 3\n";

    match result {
        TextMergeResult::Clean(merged) => {
            assert_eq!(merged, expected);
        }
        TextMergeResult::Conflict(conflict_text) => {
            panic!(
                "Expected clean merge for independent adjacent line edits, but got conflict:\n{}",
                conflict_text
            );
        }
    }
}
