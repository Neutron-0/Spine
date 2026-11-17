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

/// Simulates PostgreSQL initdb check: PGDATA must be completely empty.
pub fn verify_pgdata_is_strictly_empty(pgdata: &Path) -> Result<(), String> {
    if !pgdata.exists() {
        return Ok(());
    }
    let entries: Vec<_> = fs::read_dir(pgdata)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok())
        .collect();
    if entries.is_empty() {
        Ok(())
    } else {
        let names: Vec<String> = entries
            .into_iter()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        Err(format!(
            "initdb: error: directory \"{}\" exists but is not empty. Found entries: {}",
            pgdata.display(),
            names.join(", ")
        ))
    }
}

/// Simulates extracting an archive into a target volume without overlaying live data,
/// archiving existing data in an external rollback directory outside the volume mount.
pub fn restore_volume_clean(
    volume_root: &Path,
    rollback_parent: &Path,
    volume_name: &str,
    archive_files: &[(&str, &[u8])],
) -> Result<PathBuf, std::io::Error> {
    let rollback_dir = rollback_parent.join(format!("{}_rollback", volume_name));
    fs::create_dir_all(&rollback_dir)?;

    // Move existing volume contents to external rollback_dir, leaving volume_root strictly empty
    if volume_root.exists() {
        for entry in fs::read_dir(volume_root)? {
            let entry = entry?;
            let dest = rollback_dir.join(entry.file_name());
            fs::rename(entry.path(), dest)?;
        }
    } else {
        fs::create_dir_all(volume_root)?;
    }

    // Extract archive contents into now strictly clean volume root
    for (rel_path, content) in archive_files {
        let target = volume_root.join(rel_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&target, content)?;
    }

    Ok(rollback_dir)
}

/// Commits the restore by purging the external rollback snapshot.
pub fn commit_restore_volume(rollback_dir: &Path) -> Result<(), std::io::Error> {
    if rollback_dir.exists() {
        fs::remove_dir_all(rollback_dir)?;
    }
    Ok(())
}

/// Rolls back by clearing failed extraction and restoring preserved files from external rollback dir.
pub fn rollback_restore_volume(volume_root: &Path, rollback_dir: &Path) -> Result<(), std::io::Error> {
    // Clear failed extraction from volume_root
    if volume_root.exists() {
        for entry in fs::read_dir(volume_root)? {
            let entry = entry?;
            if entry.path().is_dir() {
                fs::remove_dir_all(entry.path())?;
            } else {
                fs::remove_file(entry.path())?;
            }
        }
    }

    // Move preserved files back from external rollback dir
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

/// Adapts `CREATE ROLE` statements from pg_dumpall into idempotent PL/pgSQL blocks
pub fn adapt_pg_dumpall_roles(sql_dump: &str) -> String {
    let mut output = Vec::new();
    for line in sql_dump.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("CREATE ROLE ") && trimmed.ends_with(';') {
            let inner = &trimmed[..trimmed.len() - 1]; // strip trailing ';'
            output.push(format!(
                "DO $$ BEGIN {}; EXCEPTION WHEN duplicate_object THEN NULL; END $$;",
                inner
            ));
        } else {
            output.push(line.to_string());
        }
    }
    output.join("\n")
}

#[test]
fn test_postgres_fresh_data_directory_must_be_strictly_empty() {
    let temp = tempdir().unwrap();
    let pgdata = temp.path().join("pgdata");
    fs::create_dir_all(&pgdata).unwrap();

    // Case 1: pgdata contains .rollback_snapshot inside it -> initdb verification MUST FAIL
    fs::create_dir(pgdata.join(".rollback_snapshot")).unwrap();
    let err = verify_pgdata_is_strictly_empty(&pgdata).unwrap_err();
    assert!(err.contains("exists but is not empty"));
    assert!(err.contains(".rollback_snapshot"));

    // Case 2: Clean pgdata by moving contents to external rollback directory
    let external_rollback = temp.path().join("external_rollback");
    fs::create_dir_all(&external_rollback).unwrap();
    fs::rename(pgdata.join(".rollback_snapshot"), external_rollback.join(".rollback_snapshot")).unwrap();

    // Now pgdata is strictly empty -> initdb verification SUCCEEDS
    assert!(verify_pgdata_is_strictly_empty(&pgdata).is_ok());
}

#[test]
fn test_postgres_restore_handles_bootstrap_roles_and_fails_closed_on_real_errors() {
    let raw_dump = r#"
CREATE ROLE spine_admin WITH SUPERUSER;
CREATE ROLE spine_migrator WITH CREATEROLE;
CREATE ROLE spine_app;
CREATE TABLE users (id SERIAL PRIMARY KEY, username TEXT NOT NULL);
INSERT INTO users (username) VALUES ('admin');
"#;

    // Adapt dump using idempotent role transformation
    let adapted_dump = adapt_pg_dumpall_roles(raw_dump);
    assert!(adapted_dump.contains("DO $$ BEGIN CREATE ROLE spine_admin WITH SUPERUSER; EXCEPTION WHEN duplicate_object THEN NULL; END $$;"));
    assert!(adapted_dump.contains("DO $$ BEGIN CREATE ROLE spine_migrator WITH CREATEROLE; EXCEPTION WHEN duplicate_object THEN NULL; END $$;"));

    // Simulate SQL runner executing against a database that already has bootstrap roles
    let existing_roles = vec!["spine_admin".to_string(), "spine_migrator".to_string(), "spine_app".to_string()];

    struct SqlSimulator {
        existing_roles: Vec<String>,
        on_error_stop: bool,
    }

    impl SqlSimulator {
        fn execute_script(&self, script: &str) -> Result<(usize, usize), String> {
            let mut roles_created = 0;
            let mut tables_created = 0;

            for line in script.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("CREATE ROLE ") {
                    let role = trimmed
                        .strip_prefix("CREATE ROLE ")
                        .unwrap()
                        .split_whitespace()
                        .next()
                        .unwrap()
                        .trim_end_matches(';');
                    if self.existing_roles.contains(&role.to_string()) {
                        if self.on_error_stop {
                            return Err(format!("ERROR: role \"{}\" already exists", role));
                        }
                    } else {
                        roles_created += 1;
                    }
                } else if trimmed.starts_with("DO $$ BEGIN CREATE ROLE ") {
                    // Idempotent block catches duplicate_object!
                    let role = trimmed
                        .strip_prefix("DO $$ BEGIN CREATE ROLE ")
                        .unwrap()
                        .split_whitespace()
                        .next()
                        .unwrap()
                        .trim_end_matches(';');
                    if !self.existing_roles.contains(&role.to_string()) {
                        roles_created += 1;
                    }
                } else if trimmed.starts_with("CREATE TABLE ") {
                    tables_created += 1;
                } else if trimmed.contains("SYNTAX_ERROR_FAIL") && self.on_error_stop {
                    return Err("ERROR: syntax error at or near \"SYNTAX_ERROR_FAIL\"".to_string());
                }
            }
            Ok((roles_created, tables_created))
        }
    }

    let runner = SqlSimulator {
        existing_roles: existing_roles.clone(),
        on_error_stop: true,
    };

    // Case 1: Raw dump without role adaptation fails on duplicate role
    let raw_err = runner.execute_script(raw_dump).unwrap_err();
    assert!(raw_err.contains("role \"spine_admin\" already exists"));

    // Case 2: Adapted dump executes cleanly without duplicate role collision
    let (roles, tables) = runner.execute_script(&adapted_dump).unwrap();
    assert_eq!(roles, 0); // All 3 roles were existing bootstrap roles, caught cleanly
    assert_eq!(tables, 1); // Table was created cleanly

    // Case 3: Real SQL syntax/constraint error fails closed under ON_ERROR_STOP=1
    let broken_dump = format!("{}\nSYNTAX_ERROR_FAIL;", adapted_dump);
    let broken_err = runner.execute_script(&broken_dump).unwrap_err();
    assert!(broken_err.contains("syntax error"));
}

#[test]
fn test_automated_rollback_reverts_all_volumes_on_failure() {
    let temp = tempdir().unwrap();
    let volume_root = temp.path().join("storage_node_1");
    let rollback_parent = temp.path().join("rollbacks");
    fs::create_dir_all(&volume_root).unwrap();

    // Initial state: live files before restore
    fs::write(volume_root.join("pack_001.pack"), b"original live packfile").unwrap();
    fs::write(volume_root.join(".fencing_generation"), b"40").unwrap();

    // Incoming archive
    let archive_contents = [
        ("pack_002.pack", b"incoming corrupted packfile" as &[u8]),
        (".fencing_generation", b"42" as &[u8]),
    ];

    // Restore with external rollback directory
    let rollback_dir = restore_volume_clean(&volume_root, &rollback_parent, "node_1", &archive_contents).unwrap();

    // Active volume has new files and NO stale files
    assert!(volume_root.join("pack_002.pack").exists());
    assert!(!volume_root.join("pack_001.pack").exists());
    assert_eq!(fs::read_to_string(volume_root.join(".fencing_generation")).unwrap(), "42");

    // Simulate post-restore failure (e.g. generation check or integrity check failed) -> TRIGGER ROLLBACK!
    rollback_restore_volume(&volume_root, &rollback_dir).unwrap();

    // CRITICAL ASSERTION: Volume must be reverted exactly to original pre-restore state!
    assert!(volume_root.join("pack_001.pack").exists(), "Original packfile must be restored");
    assert!(!volume_root.join("pack_002.pack").exists(), "Corrupted extraction must be purged");
    assert_eq!(
        fs::read_to_string(volume_root.join(".fencing_generation")).unwrap(),
        "40",
        "Original fencing generation must be restored"
    );
    assert!(!rollback_dir.exists(), "Rollback directory purged after rollback completion");
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
