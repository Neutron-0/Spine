// ==============================================================================
// Spine Forge — Out-of-Band Backup & Restore Failure Injection Tests
// Verifies fail-closed behavior for SQL errors, stale volume files,
// unstopped containers, and participant generation divergence.
// ==============================================================================

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::tempdir;

#[derive(Debug, PartialEq, Eq)]
pub struct ParticipantAuditResult {
    pub unanimous: bool,
    pub consensus_generation: Option<u64>,
    pub mismatched_participants: Vec<String>,
}

/// Validates participant generation unanimity before snapshot capture.
pub fn audit_participant_generations(
    api_gen: u64,
    coordinator_gen: Option<u64>,
    node_generations: &HashMap<String, Option<u64>>,
) -> Result<u64, String> {
    let mut mismatches = Vec::new();

    match coordinator_gen {
        Some(gen) if gen == api_gen => {}
        Some(gen) => mismatches.push(format!("coordinator (gen: {}, expected: {})", gen, api_gen)),
        None => mismatches.push("coordinator (missing generation)".to_string()),
    }

    for (node, gen_opt) in node_generations {
        match gen_opt {
            Some(gen) if *gen == api_gen => {}
            Some(gen) => mismatches.push(format!("{} (gen: {}, expected: {})", node, gen, api_gen)),
            None => mismatches.push(format!("{} (missing generation)", node)),
        }
    }

    if mismatches.is_empty() {
        Ok(api_gen)
    } else {
        Err(format!(
            "FATAL: Pre-capture audit detected generation divergence across participants: {}",
            mismatches.join("; ")
        ))
    }
}

/// Verifies that all stateful containers are confirmed stopped before modifying volumes.
pub fn verify_containers_stopped(running_services: &[String]) -> Result<(), String> {
    if running_services.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "FATAL: The following services failed to stop and are still running: {}. Aborting restore to prevent data corruption.",
            running_services.join(", ")
        ))
    }
}

/// Simulates extracting an archive into a target volume without overlaying live data,
/// preserving existing data in .rollback_snapshot until committed.
pub fn restore_volume_clean(
    volume_root: &Path,
    archive_files: &[(&str, &[u8])],
) -> Result<PathBuf, std::io::Error> {
    let rollback_dir = volume_root.join(".rollback_snapshot");
    fs::create_dir_all(&rollback_dir)?;

    // Move existing volume contents to .rollback_snapshot (excluding .rollback_snapshot itself)
    for entry in fs::read_dir(volume_root)? {
        let entry = entry?;
        let file_name = entry.file_name();
        if file_name != ".rollback_snapshot" {
            let dest = rollback_dir.join(&file_name);
            fs::rename(entry.path(), dest)?;
        }
    }

    // Extract archive contents into now-clean volume root
    for (rel_path, content) in archive_files {
        let target = volume_root.join(rel_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target, content)?;
    }

    Ok(rollback_dir)
}

/// Commits the restore by purging the rollback snapshot.
pub fn commit_restore_volume(rollback_dir: &Path) -> Result<(), std::io::Error> {
    if rollback_dir.exists() {
        fs::remove_dir_all(rollback_dir)?;
    }
    Ok(())
}

/// Rolls back by restoring preserved files and removing failed extraction.
pub fn rollback_restore_volume(volume_root: &Path, rollback_dir: &Path) -> Result<(), std::io::Error> {
    // Remove newly extracted files (excluding rollback dir)
    for entry in fs::read_dir(volume_root)? {
        let entry = entry?;
        if entry.file_name() != ".rollback_snapshot" {
            if entry.path().is_dir() {
                fs::remove_dir_all(entry.path())?;
            } else {
                fs::remove_file(entry.path())?;
            }
        }
    }

    // Move rollback contents back to volume root
    if rollback_dir.exists() {
        for entry in fs::read_dir(rollback_dir)? {
            let entry = entry?;
            let dest = volume_root.join(entry.file_name());
            fs::rename(entry.path(), dest)?;
        }
        fs::remove_dir_all(rollback_dir)?;
    }
    Ok(())
}

#[test]
fn test_backup_pre_capture_audit_rejects_divergent_participant_generations() {
    let api_gen = 42;
    let mut nodes = HashMap::new();
    nodes.insert("storage-node-1".to_string(), Some(42));
    nodes.insert("storage-node-2".to_string(), Some(42));
    nodes.insert("storage-node-3".to_string(), Some(42));

    // Case 1: All participants unanimous -> PASS
    let result = audit_participant_generations(api_gen, Some(42), &nodes);
    assert_eq!(result.unwrap(), 42);

    // Case 2: Node 2 diverged to 41 -> FAIL CLOSED
    nodes.insert("storage-node-2".to_string(), Some(41));
    let err = audit_participant_generations(api_gen, Some(42), &nodes).unwrap_err();
    assert!(err.contains("storage-node-2 (gen: 41, expected: 42)"));

    // Case 3: Coordinator diverged to 40 -> FAIL CLOSED
    nodes.insert("storage-node-2".to_string(), Some(42));
    let err = audit_participant_generations(api_gen, Some(40), &nodes).unwrap_err();
    assert!(err.contains("coordinator (gen: 40, expected: 42)"));

    // Case 4: Node 3 missing generation file -> FAIL CLOSED
    nodes.insert("storage-node-3".to_string(), None);
    let err = audit_participant_generations(api_gen, Some(42), &nodes).unwrap_err();
    assert!(err.contains("storage-node-3 (missing generation)"));
}

#[test]
fn test_restore_fails_closed_when_containers_refuse_to_stop() {
    // Case 1: All containers stopped -> PASS
    let stopped = Vec::<String>::new();
    assert!(verify_containers_stopped(&stopped).is_ok());

    // Case 2: Ingress and API still running -> FAIL CLOSED
    let unstopped = vec!["ingress".to_string(), "spine-api".to_string()];
    let err = verify_containers_stopped(&unstopped).unwrap_err();
    assert!(err.contains("ingress, spine-api"));
    assert!(err.contains("Aborting restore to prevent data corruption"));

    // Case 3: Storage node hung / refusing SIGTERM -> FAIL CLOSED
    let hung_storage = vec!["storage-node-2".to_string()];
    let err2 = verify_containers_stopped(&hung_storage).unwrap_err();
    assert!(err2.contains("storage-node-2"));
}

#[test]
fn test_volume_restore_clearing_prevents_stale_files_and_supports_rollback() {
    let temp = tempdir().unwrap();
    let volume_root = temp.path().join("storage_node_1");
    fs::create_dir_all(&volume_root).unwrap();

    // 1. Target volume initially contains live files AND a stale deleted packfile
    let active_pack = volume_root.join("pack_001.pack");
    let stale_pack = volume_root.join("stale_deleted_pack.pack");
    let fencing_file = volume_root.join(".fencing_generation");
    fs::write(&active_pack, b"pack 1 original").unwrap();
    fs::write(&stale_pack, b"this file was deleted in production before backup").unwrap();
    fs::write(&fencing_file, b"40").unwrap();

    // 2. Incoming backup only contains pack_002 and fencing generation 42
    let archive_contents = [
        ("pack_002.pack", b"pack 2 from backup" as &[u8]),
        (".fencing_generation", b"42" as &[u8]),
    ];

    // 3. Restore with volume clearing
    let rollback_dir = restore_volume_clean(&volume_root, &archive_contents).unwrap();

    // CRITICAL ASSERTION: The active volume MUST NOT contain the stale deleted packfile!
    assert!(volume_root.join("pack_002.pack").exists());
    assert!(!volume_root.join("stale_deleted_pack.pack").exists(), "Stale file must be removed");
    assert!(!volume_root.join("pack_001.pack").exists(), "Old packfile must be replaced");
    assert_eq!(
        fs::read_to_string(volume_root.join(".fencing_generation")).unwrap(),
        "42"
    );

    // Rollback snapshot preserved original files safely
    assert!(rollback_dir.join("stale_deleted_pack.pack").exists());
    assert!(rollback_dir.join("pack_001.pack").exists());
    assert_eq!(
        fs::read_to_string(rollback_dir.join(".fencing_generation")).unwrap(),
        "40"
    );

    // 4. Test Commit: purging rollback snapshot
    commit_restore_volume(&rollback_dir).unwrap();
    assert!(!rollback_dir.exists(), "Rollback directory must be purged on commit");
    assert!(volume_root.join("pack_002.pack").exists());
}

#[test]
fn test_volume_restore_rollback_on_simulated_verification_failure() {
    let temp = tempdir().unwrap();
    let volume_root = temp.path().join("coordinator_vol");
    fs::create_dir_all(&volume_root).unwrap();

    // Original state
    fs::write(volume_root.join("coordinator.state"), b"original state").unwrap();
    fs::write(volume_root.join(".coordinator_fencing_generation"), b"30").unwrap();

    // Restore incoming archive
    let archive_contents = [
        ("coordinator.state", b"restored corrupted state" as &[u8]),
        (".coordinator_fencing_generation", b"35" as &[u8]),
    ];
    let rollback_dir = restore_volume_clean(&volume_root, &archive_contents).unwrap();

    // Simulate verification check failing -> Trigger rollback!
    rollback_restore_volume(&volume_root, &rollback_dir).unwrap();

    // CRITICAL ASSERTION: Volume must be restored exactly to original state!
    assert_eq!(
        fs::read_to_string(volume_root.join("coordinator.state")).unwrap(),
        "original state"
    );
    assert_eq!(
        fs::read_to_string(volume_root.join(".coordinator_fencing_generation")).unwrap(),
        "30"
    );
    assert!(!rollback_dir.exists());
}

#[test]
fn test_postgres_restore_fails_closed_on_duplicate_objects_or_sql_error() {
    // Simulates SQL script validation with ON_ERROR_STOP semantics
    struct MockSqlRunner {
        on_error_stop: bool,
    }

    impl MockSqlRunner {
        fn execute_dump(&self, statements: &[&str], existing_objects: &[&str]) -> Result<usize, String> {
            let mut executed = 0;
            for stmt in statements {
                for existing in existing_objects {
                    if self.on_error_stop
                        && (stmt.contains(&format!("CREATE ROLE {}", existing))
                            || stmt.contains(&format!("CREATE DATABASE {}", existing))
                            || stmt.contains(&format!("CREATE TABLE {}", existing)))
                    {
                        return Err(format!(
                            "ERROR: relation or role '{}' already exists. Aborting under ON_ERROR_STOP=1.",
                            existing
                        ));
                    }
                }
                executed += 1;
            }
            Ok(executed)
        }
    }

    let dump_statements = [
        "CREATE ROLE spine_admin WITH SUPERUSER;",
        "CREATE DATABASE spine_forge;",
        "CREATE TABLE users (id SERIAL PRIMARY KEY, username TEXT);",
    ];

    // Case 1: Existing database contains duplicate objects without clean volume -> ON_ERROR_STOP fails closed!
    let existing_objects = ["spine_admin", "spine_forge"];
    let strict_runner = MockSqlRunner { on_error_stop: true };
    let err = strict_runner
        .execute_dump(&dump_statements, &existing_objects)
        .unwrap_err();
    assert!(err.contains("already exists"));
    assert!(err.contains("ON_ERROR_STOP=1"));

    // Case 2: Fresh database (empty volume) -> ON_ERROR_STOP executes cleanly
    let clean_objects = [];
    let clean_result = strict_runner.execute_dump(&dump_statements, &clean_objects);
    assert_eq!(clean_result.unwrap(), 3);
}
