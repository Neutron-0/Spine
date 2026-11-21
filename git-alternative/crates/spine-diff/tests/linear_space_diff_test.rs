use spine_diff::{compute_line_diff, DiffOp};

#[test]
fn test_large_diff_efficiency() {
    // 5000 lines identical with changes in the middle
    let mut old_lines = Vec::new();
    let mut new_lines = Vec::new();

    for i in 0..5000 {
        if i == 2500 {
            old_lines.push("original middle line");
            new_lines.push("replaced middle line 1");
            new_lines.push("replaced middle line 2");
        } else {
            let line = format!("line {}", i);
            old_lines.push(Box::leak(line.clone().into_boxed_str()) as &'static str);
            new_lines.push(Box::leak(line.into_boxed_str()) as &'static str);
        }
    }

    let old_text = old_lines.join("\n");
    let new_text = new_lines.join("\n");

    let start = std::time::Instant::now();
    let ops = compute_line_diff(&old_text, &new_text);
    let duration = start.elapsed();

    // Must finish quickly (< 500ms) without allocating huge matrices
    assert!(
        duration.as_millis() < 500,
        "Diff took too long: {:?}",
        duration
    );

    let has_add = ops.iter().any(|op| match op {
        DiffOp::Addition(s) => s == "replaced middle line 1",
        _ => false,
    });
    let has_del = ops.iter().any(|op| match op {
        DiffOp::Deletion(s) => s == "original middle line",
        _ => false,
    });

    assert!(has_add, "Expected addition of middle line 1");
    assert!(has_del, "Expected deletion of original middle line");
}

#[test]
fn test_diff_accuracy_interleaved() {
    let old_text = "A\nB\nC\nD\nE\nF\nG\n";
    let new_text = "A\nX\nC\nE\nY\nG\nZ\n";

    let ops = compute_line_diff(old_text, new_text);

    let mut reconstructed = Vec::new();
    for op in &ops {
        match op {
            DiffOp::Context(l) | DiffOp::Addition(l) => {
                reconstructed.push(l.as_str());
            }
            DiffOp::Deletion(_) => {}
        }
    }

    assert_eq!(reconstructed, vec!["A", "X", "C", "E", "Y", "G", "Z"]);
}
