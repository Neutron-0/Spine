use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_coordinator_cli_repair_generation_flags() {
    let coordinator_bin = env!("CARGO_BIN_EXE_spine-coordinator");
    let temp = tempdir().unwrap();
    let root = temp.path();
    let coord_file = root.join("coordinator.state");
    let fencing_file = root.join(".coordinator_fencing_generation");

    // 1. First run with --repair-generation without --expected-gen reports current generation (defaults to 1)
    let output = Command::new(coordinator_bin)
        .arg("--store-file")
        .arg(&coord_file)
        .arg("--repair-generation")
        .output()
        .expect("failed to execute coordinator binary");

    assert!(output.status.success(), "Exit status was {:?}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Current coordinator fencing generation: 1"),
        "stdout was: {}",
        stdout
    );
    assert_eq!(
        std::fs::read_to_string(&fencing_file).unwrap().trim(),
        "1"
    );

    // 2. Reconcile to generation 50 using flag format
    let output2 = Command::new(coordinator_bin)
        .arg("--store-file")
        .arg(&coord_file)
        .arg("--repair-generation")
        .arg("--expected-gen")
        .arg("50")
        .output()
        .expect("failed to execute coordinator binary");

    assert!(output2.status.success(), "Exit status was {:?}", output2.status);
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    assert!(
        stdout2.contains("Successfully reconciled coordinator fencing generation from 1 to 50."),
        "stdout was: {}",
        stdout2
    );
    assert_eq!(
        std::fs::read_to_string(&fencing_file).unwrap().trim(),
        "50"
    );

    // 3. Idempotent re-run with expected-gen 50
    let output3 = Command::new(coordinator_bin)
        .arg("--store-file")
        .arg(&coord_file)
        .arg("--repair-generation")
        .arg("--expected-gen")
        .arg("50")
        .output()
        .expect("failed to execute coordinator binary");

    assert!(output3.status.success(), "Exit status was {:?}", output3.status);
    let stdout3 = String::from_utf8_lossy(&output3.stdout);
    assert!(
        stdout3.contains("already aligned with expected generation 50"),
        "stdout was: {}",
        stdout3
    );
    assert_eq!(
        std::fs::read_to_string(&fencing_file).unwrap().trim(),
        "50"
    );

    // 4. Regressing to generation 40 fails closed with monotonicity error
    let output4 = Command::new(coordinator_bin)
        .arg("--store-file")
        .arg(&coord_file)
        .arg("--repair-generation")
        .arg("--expected-gen")
        .arg("40")
        .output()
        .expect("failed to execute coordinator binary");

    assert!(!output4.status.success(), "Command should have failed");
    let stderr4 = String::from_utf8_lossy(&output4.stderr);
    assert!(
        stderr4.contains("Monotonicity violation"),
        "stderr was: {}",
        stderr4
    );
    assert_eq!(
        std::fs::read_to_string(&fencing_file).unwrap().trim(),
        "50"
    );

    // 5. Reconcile using subcommand format: `spine-coordinator repair-generation --expected-gen 75`
    let output5 = Command::new(coordinator_bin)
        .arg("--store-file")
        .arg(&coord_file)
        .arg("repair-generation")
        .arg("--expected-gen")
        .arg("75")
        .output()
        .expect("failed to execute coordinator binary");

    assert!(output5.status.success(), "Exit status was {:?}", output5.status);
    let stdout5 = String::from_utf8_lossy(&output5.stdout);
    assert!(
        stdout5.contains("Successfully reconciled coordinator fencing generation from 50 to 75."),
        "stdout was: {}",
        stdout5
    );
    assert_eq!(
        std::fs::read_to_string(&fencing_file).unwrap().trim(),
        "75"
    );
}
