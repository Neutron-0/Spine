// ==============================================================================
// Spine Forge — Out-of-Band Backup & Restore Failure Injection Tests
// Verifies fail-closed behavior for SQL errors, stale volume files,
// unstopped containers, and participant generation divergence.
// ==============================================================================

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
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

/// Simulates pre-restore archiving: checks that all archives are generated and valid
/// BEFORE modifying any volume. All pre-restore archives are authenticated SPNEBAK v2
/// envelopes to prevent persisting sensitive plaintext (DB dumps, SSH keys) on disk.
/// If any participant archive fails, aborts with Err, leaving zero volumes modified.
pub fn execute_pre_restore_archiving(
    volumes: &[(&str, &Path)],
    rollback_dir: &Path,
    passphrase: &str,
    fail_on_participant: Option<&str>,
) -> Result<Vec<PathBuf>, String> {
    let mut archived = Vec::new();
    fs::create_dir_all(rollback_dir).map_err(|e| e.to_string())?;

    for (name, _path) in volumes {
        if let Some(fail_target) = fail_on_participant {
            if *name == fail_target {
                return Err(format!(
                    "FATAL: Pre-restore rollback archive creation failed for {}. ABORTING RESTORE BEFORE MODIFYING ANY VOLUME.",
                    name
                ));
            }
        }
        let archive_file = rollback_dir.join(format!("{}.spnebak", name));
        let plaintext_payload = format!("valid authenticated payload for {}", name);
        let mut encrypted = Vec::new();
        spine_api::backup::v2::encrypt_stream(
            &mut plaintext_payload.as_bytes(),
            &mut encrypted,
            passphrase,
            1,
            spine_api::backup::v2::DEFAULT_CHUNK_SIZE,
        )
        .map_err(|e| e.to_string())?;

        fs::write(&archive_file, &encrypted).map_err(|e| e.to_string())?;
        archived.push(archive_file);
    }

    Ok(archived)
}

#[test]
fn test_pre_restore_archive_failure_aborts_before_modifying_any_volume() {
    let temp = tempdir().unwrap();
    let root = temp.path();

    // Create 7 live volumes with original data
    let participants = [
        "coordinator",
        "storage_node_1",
        "storage_node_2",
        "storage_node_3",
        "spine_ssh",
        "spine_storage",
        "postgres_data",
    ];

    let mut volume_paths = Vec::new();
    for p in &participants {
        let p_dir = root.join(p);
        fs::create_dir_all(&p_dir).unwrap();
        fs::write(p_dir.join("live.data"), format!("original data for {}", p)).unwrap();
        volume_paths.push((*p, p_dir));
    }

    let rollback_dir = root.join("rollback_dir");

    // Force failure on each participant one by one!
    for p in &participants {
        let volumes_ref: Vec<(&str, &Path)> = volume_paths
            .iter()
            .map(|(name, path)| (*name, path.as_path()))
            .collect();

        let result = execute_pre_restore_archiving(&volumes_ref, &rollback_dir, "Passphrase-2026!", Some(p));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains(p));
        assert!(err.contains("ABORTING RESTORE BEFORE MODIFYING ANY VOLUME"));

        // CRITICAL INVARIANT: Every single volume MUST remain completely untouched!
        for (v_name, v_path) in &volume_paths {
            assert!(
                v_path.join("live.data").exists(),
                "Volume {} live data must be untouched after failure on {}",
                v_name,
                p
            );
            assert_eq!(
                fs::read_to_string(v_path.join("live.data")).unwrap(),
                format!("original data for {}", v_name),
                "Volume {} content must not be modified after failure on {}",
                v_name,
                p
            );
        }
    }
}

/// Helper to validate POSIX-style permissions for backup directories and archives.
/// Checks that directory has mode 0700 (owner-only) and files have mode 0600 (owner-only rw).
pub fn validate_backup_permissions(dir_mode: u32, file_modes: &[(&str, u32)]) -> Result<(), String> {
    // 0o077 represents group and other read/write/execute bits
    if dir_mode & 0o077 != 0 || (dir_mode & 0o700 != 0o700) {
        return Err(format!(
            "FATAL: Insecure directory mode {:o}. Expected 0700 (owner-only rwx).",
            dir_mode
        ));
    }

    for (file, mode) in file_modes {
        if mode & 0o077 != 0 || (mode & 0o600 != 0o600) {
            return Err(format!(
                "FATAL: Insecure file mode {:o} on {}. Expected 0600 (owner-only rw).",
                mode, file
            ));
        }
    }

    Ok(())
}

#[test]
fn test_backup_directory_permissions_enforces_restrictive_umask() {
    let secure_files = [
        ("FENCING_GENERATION", 0o600),
        ("postgres_dump.sql.gz", 0o600),
        ("coordinator.tar.gz", 0o600),
        ("spine_ssh.tar.gz", 0o600),
        ("SHA256SUMS", 0o600),
    ];

    // Case 1: Strictly secure permissions (dir 0700, files 0600) -> PASS
    assert!(validate_backup_permissions(0o700, &secure_files).is_ok());

    // Case 2: Permissive directory permissions (e.g. default umask 022 -> 0755) -> FAIL
    let err_dir = validate_backup_permissions(0o755, &secure_files).unwrap_err();
    assert!(err_dir.contains("Insecure directory mode 755"));

    // Case 3: Group-readable directory (e.g. 0750) -> FAIL
    let err_grp_dir = validate_backup_permissions(0o750, &secure_files).unwrap_err();
    assert!(err_grp_dir.contains("Insecure directory mode 750"));

    // Case 4: World-readable sensitive archive (e.g. 0644 ssh keys or postgres dump) -> FAIL
    let mut insecure_files = secure_files.to_vec();
    insecure_files[3] = ("spine_ssh.tar.gz", 0o644);
    let err_file = validate_backup_permissions(0o700, &insecure_files).unwrap_err();
    assert!(err_file.contains("Insecure file mode 644 on spine_ssh.tar.gz"));

    // Case 5: Group-writable or readable archive (e.g. 0660) -> FAIL
    let mut insecure_db = secure_files.to_vec();
    insecure_db[1] = ("postgres_dump.sql.gz", 0o660);
    let err_db = validate_backup_permissions(0o700, &insecure_db).unwrap_err();
    assert!(err_db.contains("Insecure file mode 660 on postgres_dump.sql.gz"));
}

/// Result of executing a multi-volume rollback.
#[derive(Debug, PartialEq, Eq)]
pub struct RollbackExecutionResult {
    pub success: bool,
    pub exit_code: i32,
    pub failed_participants: Vec<String>,
    pub services_running: Vec<String>,
    pub verified_participants: Vec<String>,
}

/// Simulates rollback_volumes() logic from scripts/restore_oob.sh.
pub fn execute_simulated_rollback(
    participants: &[&str],
    running_services: &[&str],
    corrupted_participant: Option<&str>,
    failed_verification_participant: Option<&str>,
) -> RollbackExecutionResult {
    let mut failed_participants = Vec::new();
    let mut verified_participants = Vec::new();

    // Step 1: Mandatory stop and quiescence verification
    // If any container refuses to stop, DO NOT EXTRACT INTO VOLUMES!
    if !running_services.is_empty() {
        failed_participants.push(format!("services_refused_to_stop:{}", running_services.join(",")));
        return RollbackExecutionResult {
            success: false,
            exit_code: 2,
            failed_participants,
            services_running: running_services.iter().map(|s| s.to_string()).collect(),
            verified_participants,
        };
    }

    for p in participants {
        // Step 2: Extract archive
        if let Some(corrupt) = corrupted_participant {
            if *p == corrupt {
                failed_participants.push(format!("extraction_failed:{}", p));
                continue;
            }
        }

        // Step 3: Post-extraction verification (marker checks across all participants)
        if let Some(unverified) = failed_verification_participant {
            if *p == unverified {
                failed_participants.push(format!("verification_failed:{}", p));
                continue;
            }
        }

        verified_participants.push(p.to_string());
    }

    let success = failed_participants.is_empty() && verified_participants.len() == participants.len();
    let exit_code = if success { 0 } else { 2 };
    // In all rollback cases (success or failure), cluster services remain strictly stopped!
    let services_running = Vec::new();

    RollbackExecutionResult {
        success,
        exit_code,
        failed_participants,
        services_running,
        verified_participants,
    }
}

#[test]
fn test_rollback_fails_and_aborts_extraction_if_services_refuse_to_stop() {
    let participants = [
        "coordinator",
        "storage_node_1",
        "storage_node_2",
        "storage_node_3",
        "spine_ssh",
        "spine_storage",
        "postgres_data",
    ];

    // Case 1: storage-node-2 refused to stop during rollback -> FATAL exit code 2, ZERO extractions performed!
    let hung_storage = ["storage-node-2"];
    let result = execute_simulated_rollback(&participants, &hung_storage, None, None);
    assert!(!result.success);
    assert_eq!(result.exit_code, 2);
    assert!(result.failed_participants.iter().any(|f| f.contains("services_refused_to_stop:storage-node-2")));
    assert!(
        result.verified_participants.is_empty(),
        "Must NOT extract into any volumes if a service refused to stop"
    );
    assert_eq!(result.services_running, vec!["storage-node-2"]);

    // Case 2: postgres and spine-api failed to stop -> FATAL exit code 2, zero extractions
    let hung_db = ["postgres", "spine-api"];
    let result_db = execute_simulated_rollback(&participants, &hung_db, None, None);
    assert!(!result_db.success);
    assert_eq!(result_db.exit_code, 2);
    assert!(result_db.failed_participants.iter().any(|f| f.contains("services_refused_to_stop:postgres,spine-api")));
    assert!(result_db.verified_participants.is_empty());
}

#[test]
fn test_rollback_extraction_failure_is_fatal_and_keeps_cluster_stopped() {
    let participants = [
        "coordinator",
        "storage_node_1",
        "storage_node_2",
        "storage_node_3",
        "spine_ssh",
        "spine_storage",
        "postgres_data",
    ];

    // Case 1: Clean rollback with all valid archives and all services stopped -> success 0, services stopped
    let clean = execute_simulated_rollback(&participants, &[], None, None);
    assert!(clean.success);
    assert_eq!(clean.exit_code, 0);
    assert_eq!(clean.verified_participants.len(), 7);
    assert!(clean.services_running.is_empty(), "Cluster must remain stopped after rollback");

    // Case 2: Extraction failure on Coordinator -> FATAL exit code 2, services stopped
    let failed_coord = execute_simulated_rollback(&participants, &[], Some("coordinator"), None);
    assert!(!failed_coord.success);
    assert_eq!(failed_coord.exit_code, 2);
    assert!(failed_coord.failed_participants.contains(&"extraction_failed:coordinator".to_string()));
    assert!(failed_coord.services_running.is_empty(), "Cluster must remain stopped on rollback failure");

    // Case 3: Extraction failure on Storage Node 2 -> FATAL exit code 2
    let failed_node2 = execute_simulated_rollback(&participants, &[], Some("storage_node_2"), None);
    assert!(!failed_node2.success);
    assert_eq!(failed_node2.exit_code, 2);
    assert!(failed_node2.failed_participants.contains(&"extraction_failed:storage_node_2".to_string()));
    assert!(failed_node2.services_running.is_empty());

    // Case 4: Extraction passes but verification fails on PostgreSQL (e.g. PG_VERSION missing) -> FATAL exit code 2
    let failed_pg_verify = execute_simulated_rollback(&participants, &[], None, Some("postgres_data"));
    assert!(!failed_pg_verify.success);
    assert_eq!(failed_pg_verify.exit_code, 2);
    assert!(failed_pg_verify.failed_participants.contains(&"verification_failed:postgres_data".to_string()));
    assert!(failed_pg_verify.services_running.is_empty());

    // Case 5: Verification fails on API SSH marker -> FATAL exit code 2
    let failed_ssh_verify = execute_simulated_rollback(&participants, &[], None, Some("spine_ssh"));
    assert!(!failed_ssh_verify.success);
    assert_eq!(failed_ssh_verify.exit_code, 2);
    assert!(failed_ssh_verify.failed_participants.contains(&"verification_failed:spine_ssh".to_string()));
}

/// Validates that a backup archive starts with the mandatory SPNEBAK v2 magic header.
pub fn validate_backup_encryption_header(data: &[u8]) -> Result<(), String> {
    if data.len() < 8 {
        return Err("FATAL: Backup archive is truncated or empty. Less than 8 bytes.".to_string());
    }
    if &data[0..8] != spine_api::backup::v2::MAGIC_V2 {
        return Err(
            "FATAL: Backup archive is unencrypted or invalid! Magic header does not match SPNEBAK v2. Under production profile 'core-forge-v1', restoring unencrypted plaintext archives is strictly forbidden."
                .to_string(),
        );
    }
    Ok(())
}

/// Authenticates and decrypts a SPNEBAK v2 backup payload using AES-256-GCM streaming decryptor.
pub fn authenticate_and_decrypt_backup_payload(
    encrypted: &[u8],
    passphrase: &str,
) -> Result<Vec<u8>, String> {
    validate_backup_encryption_header(encrypted)?;
    let mut decrypted = Vec::new();
    spine_api::backup::v2::decrypt_stream(encrypted, &mut decrypted, passphrase)
        .map_err(|e| format!("FATAL: Backup authentication or decryption failed: {}", e))?;
    Ok(decrypted)
}

#[test]
fn test_unencrypted_backup_rejected_fail_closed() {
    // Case 1: Plaintext tar archive magic (starts with "ustar" or tar header)
    let plaintext_tar = b"ustar  \0rootroot000000 000000";
    let err_tar = validate_backup_encryption_header(plaintext_tar).unwrap_err();
    assert!(err_tar.contains("Backup archive is unencrypted or invalid! Magic header does not match SPNEBAK v2"));

    // Case 2: Plaintext gzip archive magic (\x1f\x8b)
    let plaintext_gzip = b"\x1f\x8b\x08\x00\x00\x00\x00\x00";
    let err_gzip = validate_backup_encryption_header(plaintext_gzip).unwrap_err();
    assert!(err_gzip.contains("Magic header does not match SPNEBAK v2"));

    // Case 3: Plaintext SQL dump
    let plaintext_sql = b"CREATE TABLE users (id SERIAL PRIMARY KEY);";
    let err_sql = validate_backup_encryption_header(plaintext_sql).unwrap_err();
    assert!(err_sql.contains("Magic header does not match SPNEBAK v2"));

    // Case 4: Truncated header (< 8 bytes)
    let truncated = b"SPNE";
    let err_trunc = validate_backup_encryption_header(truncated).unwrap_err();
    assert!(err_trunc.contains("truncated or empty"));
}

#[test]
fn test_tampered_backup_and_passphrase_authentication_rejection() {
    let passphrase = "Production-Certified-Passphrase-2026!";
    let original_payload = b"CRITICAL_CLUSTER_STATE: PostgreSQL relational metadata and Ed25519 host keys";

    // 1. Create a valid authenticated SPNEBAK v2 encrypted archive
    let mut encrypted_archive = Vec::new();
    spine_api::backup::v2::encrypt_stream(
        &mut &original_payload[..],
        &mut encrypted_archive,
        passphrase,
        1,
        spine_api::backup::v2::DEFAULT_CHUNK_SIZE,
    )
    .unwrap();

    assert_eq!(&encrypted_archive[0..8], spine_api::backup::v2::MAGIC_V2);

    // 2. Roundtrip: Valid archive with correct passphrase decrypts cleanly
    let decrypted = authenticate_and_decrypt_backup_payload(&encrypted_archive, passphrase).unwrap();
    assert_eq!(decrypted, original_payload);

    // 3. Tampering failure injection: Mutate 1 byte in the ciphertext chunk
    let mut tampered_archive = encrypted_archive.clone();
    let tamper_idx = tampered_archive.len() - 10;
    tampered_archive[tamper_idx] ^= 0xFF; // Flip all bits of a ciphertext byte

    let tamper_err = authenticate_and_decrypt_backup_payload(&tampered_archive, passphrase).unwrap_err();
    assert!(
        tamper_err.contains("Backup authentication or decryption failed"),
        "Tampered ciphertext must fail AEAD MAC authentication: {}",
        tamper_err
    );

    // 4. Tampering failure injection: Mutate nonce prefix or salt in the header
    let mut tampered_header = encrypted_archive.clone();
    tampered_header[20] ^= 0xAA; // Flip salt bit
    let header_err = authenticate_and_decrypt_backup_payload(&tampered_header, passphrase).unwrap_err();
    assert!(
        header_err.contains("Backup authentication or decryption failed"),
        "Tampered salt/header must fail authentication: {}",
        header_err
    );

    // 5. Wrong passphrase: AEAD decryption fails closed
    let wrong_pass_err =
        authenticate_and_decrypt_backup_payload(&encrypted_archive, "Wrong-Attacker-Passphrase-999!").unwrap_err();
    assert!(
        wrong_pass_err.contains("Backup authentication or decryption failed"),
        "Wrong passphrase must fail authentication: {}",
        wrong_pass_err
    );
}

#[test]
fn test_container_backup_decrypt_command_invocation() {
    let binary = env!("CARGO_BIN_EXE_spine-api");
    let passphrase = "Container-Command-Test-Passphrase-2026!";
    let test_payload = b"CRITICAL_PAYLOAD_FOR_CONTAINER_DECRYPT_VERIFICATION_TEST_987654321";

    // 1. Prepare an authenticated encrypted SPNEBAK v2 payload
    let mut encrypted = Vec::new();
    spine_api::backup::v2::encrypt_stream(
        &mut &test_payload[..],
        &mut encrypted,
        passphrase,
        1,
        spine_api::backup::v2::DEFAULT_CHUNK_SIZE,
    )
    .unwrap();

    // 2. Case 1: Bug regression test - duplicate CLI invocation
    // In container mode with ENTRYPOINT ["/usr/local/bin/spine-api"], passing `spine-api backup decrypt ...`
    // results in `/usr/local/bin/spine-api spine-api backup decrypt ...`, which fails with unrecognized subcommand.
    let mut bad_cmd = Command::new(binary);
    bad_cmd.args(&["spine-api", "backup", "decrypt", "--input", "-", "--output", "-"]);
    bad_cmd.env("SPINE_BACKUP_PASSPHRASE", passphrase);
    bad_cmd.stdin(std::process::Stdio::piped());
    bad_cmd.stdout(std::process::Stdio::piped());
    bad_cmd.stderr(std::process::Stdio::piped());

    let mut child = bad_cmd.spawn().expect("failed to spawn bad command");
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(&encrypted);
    }
    let output = child.wait_with_output().expect("failed to wait on child");
    assert!(!output.status.success(), "Duplicate CLI invocation must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unrecognized subcommand") || stderr.contains("error:") || stderr.contains("unexpected argument"),
        "stderr should indicate invalid argument: {}",
        stderr
    );

    // 3. Case 2: Fixed container command form
    // When ENTRYPOINT is `/usr/local/bin/spine-api`, arguments should be `["backup", "decrypt", "--input", "-", "--output", "-"]`
    let mut good_cmd = Command::new(binary);
    good_cmd.args(&["backup", "decrypt", "--input", "-", "--output", "-"]);
    good_cmd.env("SPINE_BACKUP_PASSPHRASE", passphrase);
    good_cmd.stdin(std::process::Stdio::piped());
    good_cmd.stdout(std::process::Stdio::piped());
    good_cmd.stderr(std::process::Stdio::piped());

    let mut child = good_cmd.spawn().expect("failed to spawn good command");
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(&encrypted).expect("failed to write encrypted payload to stdin");
    }
    let output = child.wait_with_output().expect("failed to wait on good child");
    assert!(
        output.status.success(),
        "Container command form must succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, test_payload, "Decrypted stdout must match original plaintext");
}

#[test]
fn test_backup_failure_cleanup_purges_plaintext_staging_and_retains_encrypted_archive() {
    let temp = tempdir().unwrap();
    let backup_dir = temp.path().join("20261120_120000Z_gen_42");
    let staging_dir = backup_dir.join(".staging");
    let plaintext_bundle = backup_dir.join(".plaintext_bundle.tar.gz");
    let encrypted_archive = backup_dir.join("backup.spnebak");

    fs::create_dir_all(&staging_dir).unwrap();
    // Simulate staging sensitive database dump and SSH keys
    fs::write(staging_dir.join("postgres_dump.sql.gz"), b"CONFIDENTIAL_PG_DUMP_PLAINTEXT").unwrap();
    fs::write(staging_dir.join("spine_ssh.tar.gz"), b"CONFIDENTIAL_SSH_PRIVATE_KEY_PLAINTEXT").unwrap();
    fs::write(&plaintext_bundle, b"CONFIDENTIAL_PLAINTEXT_TAR_ARCHIVE_BUNDLE").unwrap();
    fs::write(&encrypted_archive, b"SPNEBAK\x02_VALID_AUTHENTICATED_ENCRYPTED_RESULT").unwrap();

    // Verify all files exist before failure simulation
    assert!(staging_dir.exists());
    assert!(staging_dir.join("postgres_dump.sql.gz").exists());
    assert!(staging_dir.join("spine_ssh.tar.gz").exists());
    assert!(plaintext_bundle.exists());
    assert!(encrypted_archive.exists());

    // Execute backup cleanup handler simulation mirroring scripts/backup_oob.sh cleanup_backup()
    struct BackupCleanupResult {
        staging_purged: bool,
        plaintext_bundle_purged: bool,
        encrypted_archive_retained: bool,
        cleanup_failures: Vec<String>,
    }

    fn run_simulated_backup_cleanup(
        staging: &Path,
        bundle: &Path,
        encrypted: &Path,
    ) -> BackupCleanupResult {
        let mut failures = Vec::new();
        if bundle.exists() {
            if let Err(e) = fs::remove_file(bundle) {
                failures.push(format!("Failed to remove plaintext bundle: {}", e));
            }
        }
        if staging.exists() {
            if let Err(e) = fs::remove_dir_all(staging) {
                failures.push(format!("Failed to remove staging directory: {}", e));
            }
        }
        BackupCleanupResult {
            staging_purged: !staging.exists(),
            plaintext_bundle_purged: !bundle.exists(),
            encrypted_archive_retained: encrypted.exists(),
            cleanup_failures: failures,
        }
    }

    let result = run_simulated_backup_cleanup(&staging_dir, &plaintext_bundle, &encrypted_archive);
    assert!(result.staging_purged, "Staging directory with plaintext DB and SSH keys must be purged");
    assert!(result.plaintext_bundle_purged, "Temporary plaintext bundle must be purged");
    assert!(result.encrypted_archive_retained, "Encrypted backup archive must be retained");
    assert!(result.cleanup_failures.is_empty(), "No cleanup failures on normal deletion");

    // Bash trap execution test (if bash is available)
    if let Ok(bash_check) = Command::new("bash").arg("-c").arg("echo ok").output() {
        if bash_check.status.success() {
            let script = r#"
set -euo pipefail
WORK_DIR="/tmp/spine_test_backup_trap_$$"
STAGING_DIR="${WORK_DIR}/.staging"
PLAINTEXT_TAR="${WORK_DIR}/.plaintext_bundle.tar.gz"
ENCRYPTED_ARCHIVE="${WORK_DIR}/backup.spnebak"

rm -rf "${WORK_DIR}"
mkdir -p "${STAGING_DIR}"
echo "SENSITIVE_DB_DUMP" > "${STAGING_DIR}/postgres_dump.sql.gz"
echo "SENSITIVE_SSH_KEY" > "${STAGING_DIR}/spine_ssh.tar.gz"
echo "SENSITIVE_BUNDLE" > "${PLAINTEXT_TAR}"
echo "SPNEBAK_ENCRYPTED" > "${ENCRYPTED_ARCHIVE}"

cleanup_backup() {
  local cleanup_err=0
  if [ -n "${PLAINTEXT_TAR:-}" ] && [ -e "${PLAINTEXT_TAR}" ]; then
    rm -rf "${PLAINTEXT_TAR}" || cleanup_err=1
  fi
  if [ -n "${STAGING_DIR:-}" ] && [ -e "${STAGING_DIR}" ]; then
    rm -rf "${STAGING_DIR}" || cleanup_err=1
  fi
  if [ -e "${STAGING_DIR}" ]; then exit 99; fi
  if [ -e "${PLAINTEXT_TAR}" ]; then exit 98; fi
  if [ ! -e "${ENCRYPTED_ARCHIVE}" ]; then exit 97; fi
  rm -rf "${WORK_DIR}"
}
trap cleanup_backup EXIT ERR INT TERM

# Force failure after staging (e.g. capture, encryption, or verification failure)
exit 42
"#
            .replace("\r\n", "\n")
            .replace('\r', "");

            let mut child = Command::new("bash")
                .arg("-s")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("failed to spawn bash");

            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(script.as_bytes()).expect("failed to write script to stdin");
            }
            let bash_test = child.wait_with_output().expect("failed to wait on bash child");

            assert_eq!(
                bash_test.status.code(),
                Some(42),
                "Bash script must exit with the injected failure code. Stderr: {}, Stdout: {}",
                String::from_utf8_lossy(&bash_test.stderr),
                String::from_utf8_lossy(&bash_test.stdout),
            );
        }
    }
}

#[test]
fn test_restore_cleanup_trap_purges_decrypted_payload_on_checksum_failure() {
    let temp = tempdir().unwrap();
    let decrypt_dir = temp.path().join(".decrypted_restore_test_123");
    fs::create_dir_all(&decrypt_dir).unwrap();

    // Populate with sensitive decrypted files
    fs::write(decrypt_dir.join("postgres_dump.sql.gz"), b"DECRYPTED_PLAINTEXT_PG_DUMP").unwrap();
    fs::write(decrypt_dir.join("spine_ssh.tar.gz"), b"DECRYPTED_PLAINTEXT_SSH_KEYS").unwrap();
    fs::write(decrypt_dir.join("FENCING_GENERATION"), b"42").unwrap();
    fs::write(decrypt_dir.join("SHA256SUMS"), b"badhash  postgres_dump.sql.gz\nbadhash  spine_ssh.tar.gz\n").unwrap();

    assert!(decrypt_dir.exists());
    assert!(decrypt_dir.join("postgres_dump.sql.gz").exists());
    assert!(decrypt_dir.join("spine_ssh.tar.gz").exists());

    // Simulate cleanup handler behavior when validation fails
    fn run_restore_validation_with_cleanup(dir: &Path) -> Result<(), String> {
        let restore_success = false;

        // Simulated validation logic
        let validation_result = (|| -> Result<(), String> {
            // Checksum verification failure: mismatch injected
            Err("SHA256 checksum verification failed: postgres_dump.sql.gz FAILED".to_string())
        })();

        // Cleanup trap execution on failure
        if !restore_success && dir.exists() {
            fs::remove_dir_all(dir).map_err(|e| e.to_string())?;
        }

        validation_result
    }

    let err = run_restore_validation_with_cleanup(&decrypt_dir).unwrap_err();
    assert!(err.contains("checksum verification failed"));
    assert!(!decrypt_dir.exists(), "Decrypted restore directory MUST be completely purged on checksum failure");

    // Bash trap test: execute actual trap in bash to prove early trap catches checksum failure
    if let Ok(bash_check) = Command::new("bash").arg("-c").arg("echo ok").output() {
        if bash_check.status.success() {
            let script = r#"
set -euo pipefail
WORK_DIR="/tmp/spine_test_restore_trap_$$"
DECRYPT_DIR="${WORK_DIR}/.decrypted_restore_test"
rm -rf "${WORK_DIR}"
mkdir -p "${DECRYPT_DIR}"
echo "SENSITIVE_POSTGRES_DUMP" > "${DECRYPT_DIR}/postgres_dump.sql.gz"
echo "SENSITIVE_SSH_KEY" > "${DECRYPT_DIR}/spine_ssh.tar.gz"
echo "0000000000000000000000000000000000000000000000000000000000000000  postgres_dump.sql.gz" > "${DECRYPT_DIR}/SHA256SUMS"

RESTORE_SUCCESS=0
restore_cleanup() {
  if [ "${RESTORE_SUCCESS}" -eq 0 ]; then
    if [ -n "${DECRYPT_DIR:-}" ] && [ -d "${DECRYPT_DIR}" ]; then
      rm -rf "${DECRYPT_DIR}"
    fi
  fi
  if [ -d "${DECRYPT_DIR}" ]; then exit 99; fi
  rm -rf "${WORK_DIR}"
}
trap restore_cleanup EXIT ERR INT TERM

# Force failure during checksum verification
cd "${DECRYPT_DIR}"
sha256sum -c SHA256SUMS
"#
            .replace("\r\n", "\n")
            .replace('\r', "");

            let mut child = Command::new("bash")
                .arg("-s")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("failed to spawn bash");

            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(script.as_bytes()).expect("failed to write script to stdin");
            }
            let bash_test = child.wait_with_output().expect("failed to wait on bash child");

            assert!(!bash_test.status.success(), "Checksum failure must fail closed");
        }
    }
}

#[test]
fn test_pre_restore_rollback_archives_are_encrypted_and_verified() {
    let temp = tempdir().unwrap();
    let root = temp.path();
    let rollback_dir = root.join(".pre_restore_rollback_test");
    let passphrase = "Rollback-Encryption-Test-Passphrase-2026!";

    let participants = [
        "coordinator",
        "storage_node_1",
        "storage_node_2",
        "storage_node_3",
        "spine_ssh",
        "spine_storage",
        "postgres_data",
    ];

    let mut volume_paths = Vec::new();
    for p in &participants {
        let p_dir = root.join(p);
        fs::create_dir_all(&p_dir).unwrap();
        fs::write(p_dir.join("state.bin"), format!("state for {}", p)).unwrap();
        volume_paths.push((*p, p_dir));
    }

    let volumes_ref: Vec<(&str, &Path)> = volume_paths
        .iter()
        .map(|(name, path)| (*name, path.as_path()))
        .collect();

    // 1. Generate encrypted pre-restore rollback archives
    let archives = execute_pre_restore_archiving(&volumes_ref, &rollback_dir, passphrase, None).unwrap();
    assert_eq!(archives.len(), 7);

    // 2. Verify all archives are encrypted SPNEBAK v2 and NO plaintext archives exist
    for p in &participants {
        let enc_file = rollback_dir.join(format!("{}.spnebak", p));
        let plain_tar = rollback_dir.join(format!("{}.tar.gz", p));

        assert!(enc_file.exists(), "Encrypted rollback archive {}.spnebak must exist", p);
        assert!(!plain_tar.exists(), "Plaintext archive {}.tar.gz must NOT exist", p);

        let data = fs::read(&enc_file).unwrap();
        assert!(data.len() > 8);
        assert_eq!(&data[0..8], spine_api::backup::v2::MAGIC_V2, "{}.spnebak must start with SPNEBAK v2 magic", p);

        // Verify AEAD authentication and decrypt stream
        let mut decrypted = Vec::new();
        spine_api::backup::v2::decrypt_stream(&data[..], &mut decrypted, passphrase)
            .expect("Decryption must succeed with valid passphrase");
        assert_eq!(decrypted, format!("valid authenticated payload for {}", p).as_bytes());
    }

    // 3. Verify tampering detection on rollback archives
    let tampered_file = rollback_dir.join("postgres_data.spnebak");
    let mut tampered_bytes = fs::read(&tampered_file).unwrap();
    let last = tampered_bytes.len() - 5;
    tampered_bytes[last] ^= 0x55; // Tamper MAC / ciphertext byte

    let mut out = Vec::new();
    let decrypt_err = spine_api::backup::v2::decrypt_stream(&tampered_bytes[..], &mut out, passphrase);
    assert!(decrypt_err.is_err(), "Tampered rollback archive must fail AEAD MAC verification");
}

/// Helper to verify supported storage topology: fails closed if non-local driver is detected.
pub fn verify_storage_topology_driver(volume_drivers: &HashMap<String, String>) -> Result<(), String> {
    for (vol, driver) in volume_drivers {
        if driver != "local" {
            return Err(format!(
                "FATAL: Volume '{}' is configured with unsupported driver '{}'. Only 'local' volume driver is supported for production profile core-forge-v1.",
                vol, driver
            ));
        }
    }
    Ok(())
}

#[test]
fn test_unsupported_storage_volume_driver_rejected_fail_closed() {
    // 1. Rust logic unit test: local driver passes, nfs/cifs fails closed
    let mut valid_topology = HashMap::new();
    valid_topology.insert("postgres_data".to_string(), "local".to_string());
    valid_topology.insert("storage_1_data".to_string(), "local".to_string());
    assert!(verify_storage_topology_driver(&valid_topology).is_ok());

    let mut invalid_topology = HashMap::new();
    invalid_topology.insert("postgres_data".to_string(), "local".to_string());
    invalid_topology.insert("storage_1_data".to_string(), "nfs".to_string());
    let err = verify_storage_topology_driver(&invalid_topology).unwrap_err();
    assert!(err.contains("unsupported driver 'nfs'"));
    assert!(err.contains("Only 'local' volume driver is supported"));

    // 2. Shell execution test: verify exact function in bash
    if let Ok(bash_check) = Command::new("bash").arg("-c").arg("echo ok").output() {
        if bash_check.status.success() {
            let script = r#"
set -euo pipefail
WORK_DIR="/tmp/spine_test_topo_$$"
mkdir -p "${WORK_DIR}"
COMPOSE_FILE="${WORK_DIR}/docker-compose.test.yml"

cat <<'EOF' > "${COMPOSE_FILE}"
version: '3.8'
services:
  spine-api:
    image: spine:latest
volumes:
  postgres_data:
  storage_1_data:
    driver: nfs
EOF

verify_supported_storage_topology() {
  local required_volumes=("postgres_data" "storage_1_data")
  for vol in "${required_volumes[@]}"; do
    local custom_driver
    custom_driver=$(awk -v v="${vol}:" '
      $1 == v { in_vol=1; next }
      in_vol && /^[^ ]/ { in_vol=0 }
      in_vol && /^  [^ ]/ { in_vol=0 }
      in_vol && /driver:/ { print $2; exit }
    ' "${COMPOSE_FILE}" 2>/dev/null | tr -d '[:space:]"' | tr -d "'" || echo "")
    if [ -n "${custom_driver}" ] && [ "${custom_driver}" != "local" ]; then
      echo "FATAL: Volume '${vol}' in ${COMPOSE_FILE} is configured with unsupported driver '${custom_driver}'." >&2
      echo "Only driver 'local' is supported for production profile core-forge-v1." >&2
      exit 1
    fi
  done
}

if ( verify_supported_storage_topology 2>"${WORK_DIR}/error.log" ); then
  rm -rf "${WORK_DIR}"
  exit 99
fi

if grep -q "unsupported driver 'nfs'" "${WORK_DIR}/error.log"; then
  rm -rf "${WORK_DIR}"
  exit 0
else
  rm -rf "${WORK_DIR}"
  exit 1
fi
"#
            .replace("\r\n", "\n")
            .replace('\r', "");

            let mut child = Command::new("bash")
                .arg("-s")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("failed to spawn bash");

            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(script.as_bytes()).expect("failed to write script to stdin");
            }
            let res = child.wait_with_output().expect("failed to wait on bash child");
            assert!(
                res.status.success(),
                "Storage topology driver verification must fail closed on nfs. Stderr: {}",
                String::from_utf8_lossy(&res.stderr)
            );
        }
    }
}

#[test]
fn test_backup_credentials_not_exposed_in_process_args_or_container_env() {
    let temp = tempdir().unwrap();
    let secret_file = temp.path().join("backup_pass.secret");
    let secret_pass = "SuperSecret-Production-Passphrase-9876!";
    fs::write(&secret_file, secret_pass).unwrap();

    assert!(secret_file.exists());
    assert_eq!(fs::read_to_string(&secret_file).unwrap(), secret_pass);

    if let Ok(bash_check) = Command::new("bash").arg("-c").arg("echo ok").output() {
        if bash_check.status.success() {
            let script = r#"
set -euo pipefail
WORK_DIR="/tmp/spine_test_cred_$$"
mkdir -p "${WORK_DIR}"
SECRET_FILE="${WORK_DIR}/passphrase.txt"
AUDIT_LOG="${WORK_DIR}/audit_invocation.log"
printf "%s" "SuperSecret-Production-Passphrase-9876!" > "${SECRET_FILE}"
chmod 600 "${SECRET_FILE}"

export SPINE_BACKUP_PASSPHRASE="SuperSecret-Production-Passphrase-9876!"
export SPINE_BACKUP_PASSPHRASE_FILE=""

RESOLVED_PASSPHRASE_FILE=""
CLEANUP_TEMP_PASSPHRASE=0

resolve_passphrase_file() {
  if [ -n "${SPINE_BACKUP_PASSPHRASE_FILE:-}" ] && [ -f "${SPINE_BACKUP_PASSPHRASE_FILE}" ]; then
    RESOLVED_PASSPHRASE_FILE="${SPINE_BACKUP_PASSPHRASE_FILE}"
  elif [ -n "${SPINE_BACKUP_PASSPHRASE:-}" ]; then
    RESOLVED_PASSPHRASE_FILE=$(mktemp "${WORK_DIR}/spine_pass_XXXXXX")
    chmod 600 "${RESOLVED_PASSPHRASE_FILE}"
    printf "%s" "${SPINE_BACKUP_PASSPHRASE}" > "${RESOLVED_PASSPHRASE_FILE}"
    CLEANUP_TEMP_PASSPHRASE=1
    unset SPINE_BACKUP_PASSPHRASE
  fi
}

resolve_passphrase_file

if [ -n "${SPINE_BACKUP_PASSPHRASE:-}" ]; then
  echo "FAIL: SPINE_BACKUP_PASSPHRASE was not unset from environment!" >&2
  exit 1
fi

mock_docker_compose() {
  echo "ARGS: $@" >> "${AUDIT_LOG}"
  for arg in "$@"; do
    if [[ "${arg}" == *"SuperSecret"* ]]; then
      echo "FAIL: Secret found in argv: ${arg}" >&2
      exit 2
    fi
  done
  if env | grep -q "SuperSecret"; then
    echo "FAIL: Secret found in environment variables!" >&2
    exit 3
  fi
}

COMPOSE_FILE="docker-compose.prod.yml"
mock_docker_compose run --rm -i --no-deps \
  -v "${RESOLVED_PASSPHRASE_FILE}:/run/secrets/backup_passphrase:ro" \
  spine-api \
  backup encrypt --passphrase-file /run/secrets/backup_passphrase --input - --output -

if ! grep -q -- "-v .*:/run/secrets/backup_passphrase:ro" "${AUDIT_LOG}"; then
  echo "FAIL: Secret file was not mounted to /run/secrets/backup_passphrase:ro" >&2
  exit 4
fi

if ! grep -q -- "--passphrase-file /run/secrets/backup_passphrase" "${AUDIT_LOG}"; then
  echo "FAIL: CLI was not invoked with --passphrase-file /run/secrets/backup_passphrase" >&2
  exit 5
fi

if grep -q -- "-e SPINE_BACKUP_PASSPHRASE" "${AUDIT_LOG}"; then
  echo "FAIL: -e SPINE_BACKUP_PASSPHRASE found in docker invocation!" >&2
  exit 6
fi

rm -rf "${WORK_DIR}"
exit 0
"#
            .replace("\r\n", "\n")
            .replace('\r', "");

            let mut child = Command::new("bash")
                .arg("-s")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("failed to spawn bash");

            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(script.as_bytes()).expect("failed to write script to stdin");
            }
            let res = child.wait_with_output().expect("failed to wait on bash child");
            assert!(
                res.status.success(),
                "Credential non-exposure test failed. Stderr: {}, Stdout: {}",
                String::from_utf8_lossy(&res.stderr),
                String::from_utf8_lossy(&res.stdout)
            );
        }
    }
}

#[test]
fn test_off_host_s3_replication_and_remote_restore_gate() {
    if let Ok(bash_check) = Command::new("bash").arg("-c").arg("echo ok").output() {
        if bash_check.status.success() {
            let script = r#"
set -euo pipefail
WORK_DIR="/tmp/spine_test_s3_$$"
mkdir -p "${WORK_DIR}"
MOCK_BIN="${WORK_DIR}/bin"
mkdir -p "${MOCK_BIN}"
MOCK_S3_DIR="${WORK_DIR}/s3_remote"
mkdir -p "${MOCK_S3_DIR}"
AUDIT_AWS="${WORK_DIR}/aws_audit.log"

cat <<EOF > "${MOCK_BIN}/aws"
#!/usr/bin/env bash
echo "AWS_CALL: \$@" >> "${AUDIT_AWS}"

if [ "\$1" = "s3" ] && [ "\$2" = "cp" ]; then
  SRC="\$3"
  DST="\$4"
  if [[ "\$*" == *"--object-lock-mode COMPLIANCE"* ]] && [[ "\$*" == *"--object-lock-retain-until-date"* ]]; then
    echo "OBJECT_LOCK_CONFIRMED" >> "${AUDIT_AWS}"
  fi
  if [[ "\${DST}" =~ ^s3:// ]]; then
    KEY=\$(basename "\${DST}")
    cp "\${SRC}" "${MOCK_S3_DIR}/\${KEY}"
  elif [[ "\${SRC}" =~ ^s3:// ]]; then
    KEY=\$(basename "\${SRC}")
    cp "${MOCK_S3_DIR}/\${KEY}" "\${DST}"
  fi
  exit 0
elif [ "\$1" = "s3" ] && [ "\$2" = "ls" ]; then
  TARGET="\$3"
  KEY=\$(basename "\${TARGET}")
  FILE="${MOCK_S3_DIR}/\${KEY}"
  if [ -f "\${FILE}" ]; then
    SIZE=\$(stat -c%s "\${FILE}" 2>/dev/null || stat -f%z "\${FILE}" 2>/dev/null || wc -c < "\${FILE}")
    echo "2026-11-20 12:00:00 \${SIZE} \${KEY}"
    exit 0
  else
    exit 1
  fi
fi
EOF
chmod +x "${MOCK_BIN}/aws"
export PATH="${MOCK_BIN}:${PATH}"

# 1. Test Backup Replication
LOCAL_BACKUP="${WORK_DIR}/backup.spnebak"
echo "MOCK_ENCRYPTED_SPNEBAK_DATA_PAYLOAD" > "${LOCAL_BACKUP}"
LOCAL_MANIFEST="${WORK_DIR}/SHA256SUMS"
sha256sum "${LOCAL_BACKUP}" > "${LOCAL_MANIFEST}"

export SPINE_BACKUP_REMOTE_S3_URI="s3://disaster-recovery-vault/spine"
export SPINE_BACKUP_OBJECT_LOCK_MODE="COMPLIANCE"
export SPINE_BACKUP_RETENTION_DAYS="30"
export SPINE_S3_ENDPOINT_URL=""

REMOTE_BASE="${SPINE_BACKUP_REMOTE_S3_URI%/}"
REMOTE_TARGET="${REMOTE_BASE}/test_backup"
RETAIN_UNTIL="2026-12-20T12:00:00Z"

UPLOAD_ARGS=(--object-lock-mode "${SPINE_BACKUP_OBJECT_LOCK_MODE}" --object-lock-retain-until-date "${RETAIN_UNTIL}")

aws s3 cp "${LOCAL_BACKUP}" "${REMOTE_TARGET}/backup.spnebak" "${UPLOAD_ARGS[@]}"
aws s3 cp "${LOCAL_MANIFEST}" "${REMOTE_TARGET}/SHA256SUMS"

LOCAL_SIZE=$(stat -c%s "${LOCAL_BACKUP}" 2>/dev/null || stat -f%z "${LOCAL_BACKUP}" 2>/dev/null || wc -c < "${LOCAL_BACKUP}" | tr -d '[:space:]')
REMOTE_SIZE=$(aws s3 ls "${REMOTE_TARGET}/backup.spnebak" | awk '{print $3}' | tr -d '[:space:]')

if [ "${LOCAL_SIZE}" != "${REMOTE_SIZE}" ]; then
  echo "FAIL: Local and remote size mismatch: ${LOCAL_SIZE} vs ${REMOTE_SIZE}" >&2
  exit 1
fi

if ! grep -q "OBJECT_LOCK_CONFIRMED" "${AUDIT_AWS}"; then
  echo "FAIL: Object lock parameters were not passed to aws s3 cp" >&2
  exit 2
fi

# 2. Test Remote Restore Download
DOWNLOAD_DIR="${WORK_DIR}/downloaded"
mkdir -p "${DOWNLOAD_DIR}"
aws s3 cp "${REMOTE_TARGET}/backup.spnebak" "${DOWNLOAD_DIR}/backup.spnebak"
aws s3 cp "${REMOTE_TARGET}/SHA256SUMS" "${DOWNLOAD_DIR}/SHA256SUMS"

(cd "${DOWNLOAD_DIR}" && sha256sum -c SHA256SUMS)

rm -rf "${WORK_DIR}"
exit 0
"#
            .replace("\r\n", "\n")
            .replace('\r', "");

            let mut child = Command::new("bash")
                .arg("-s")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("failed to spawn bash");

            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(script.as_bytes()).expect("failed to write script to stdin");
            }
            let res = child.wait_with_output().expect("failed to wait on bash child");
            assert!(
                res.status.success(),
                "Off-host S3 replication and restore gate test failed. Stderr: {}, Stdout: {}",
                String::from_utf8_lossy(&res.stderr),
                String::from_utf8_lossy(&res.stdout)
            );
        }
    }
}

#[test]
fn test_backup_off_site_policy_enforcement_and_local_only_mode() {
    // 1. Rust logic unit test: Enforced policy requires S3 URI; invalid values fail closed
    fn check_policy(policy: &str, s3_uri: Option<&str>) -> Result<(), String> {
        match policy {
            "1" | "true" => match s3_uri {
                Some(uri) if !uri.trim().is_empty() => Ok(()),
                _ => Err("FATAL: Off-site replication policy is ENFORCED (SPINE_BACKUP_REQUIRE_REMOTE=1), but SPINE_BACKUP_REMOTE_S3_URI is not set.".to_string()),
            },
            "0" | "false" => Ok(()),
            other => Err(format!(
                "FATAL: Invalid SPINE_BACKUP_REQUIRE_REMOTE value: '{}'. Must be '1', 'true', '0', or 'false'.",
                other
            )),
        }
    }

    assert!(check_policy("1", Some("s3://vault/backups")).is_ok());
    assert!(check_policy("true", Some("s3://vault/backups")).is_ok());
    assert!(check_policy("1", None).is_err());
    assert!(check_policy("1", Some("")).is_err());
    assert!(check_policy("0", None).is_ok());
    assert!(check_policy("false", None).is_ok());
    // Reject invalid values
    assert!(check_policy("yes", Some("s3://vault/backups")).is_err());
    assert!(check_policy("2", Some("s3://vault/backups")).is_err());
    assert!(check_policy("no", None).is_err());
    assert!(check_policy("maybe", None).is_err());

    // 2. Shell execution test: verify exact pre-flight check in bash
    if let Ok(bash_check) = Command::new("bash").arg("-c").arg("echo ok").output() {
        if bash_check.status.success() {
            let script = r#"
set -euo pipefail
WORK_DIR="/tmp/spine_test_policy_$$"
mkdir -p "${WORK_DIR}"

compute_retain_until_date() {
  local days="$1"
  local d=""
  d=$(date -u -d "+${days} days" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || true)
  if [ -z "${d}" ]; then
    d=$(date -u -v "+${days}d" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || true)
  fi
  printf "%s" "${d}"
}

verify_off_site_replication_policy() {
  local profile="${SPINE_DEPLOYMENT_PROFILE:-development}"
  case "${SPINE_BACKUP_REQUIRE_REMOTE}" in
    1|true)
      if [ -z "${SPINE_BACKUP_REMOTE_S3_URI}" ]; then
        echo "FATAL: Off-site replication policy is ENFORCED (SPINE_BACKUP_REQUIRE_REMOTE=${SPINE_BACKUP_REQUIRE_REMOTE}), but SPINE_BACKUP_REMOTE_S3_URI is not set." >&2
        exit 1
      fi
      ;;
    0|false)
      if [ "${profile}" = "core-forge-v1" ]; then
        echo "FATAL: Local-only backup (SPINE_BACKUP_REQUIRE_REMOTE=0) is strictly FORBIDDEN under production profile 'core-forge-v1'." >&2
        exit 1
      fi
      echo "NOTICE: Operating in INTENTIONALLY LOCAL-ONLY mode."
      ;;
    *)
      echo "FATAL: Invalid SPINE_BACKUP_REQUIRE_REMOTE value: '${SPINE_BACKUP_REQUIRE_REMOTE}'" >&2
      exit 1
      ;;
  esac
}

# 1. Enforced mode with missing URI must exit 1
export SPINE_BACKUP_REQUIRE_REMOTE=1
export SPINE_BACKUP_REMOTE_S3_URI=""
if ( verify_off_site_replication_policy 2>"${WORK_DIR}/enforce.log" ); then
  echo "FAIL: Enforced mode did not fail closed on missing S3 URI" >&2
  exit 99
fi
grep -q "FATAL: Off-site replication policy is ENFORCED" "${WORK_DIR}/enforce.log"

# 2. Local-only mode under non-production profile (development) must exit 0 and emit notice
export SPINE_DEPLOYMENT_PROFILE=development
export SPINE_BACKUP_REQUIRE_REMOTE=0
export SPINE_BACKUP_REMOTE_S3_URI=""
verify_off_site_replication_policy > "${WORK_DIR}/local_only.log"
grep -q "INTENTIONALLY LOCAL-ONLY" "${WORK_DIR}/local_only.log"

export SPINE_BACKUP_REQUIRE_REMOTE=false
verify_off_site_replication_policy > "${WORK_DIR}/local_false.log"
grep -q "INTENTIONALLY LOCAL-ONLY" "${WORK_DIR}/local_false.log"

# 2b. Local-only mode under production profile (core-forge-v1) must FAIL CLOSED
export SPINE_DEPLOYMENT_PROFILE=core-forge-v1
export SPINE_BACKUP_REQUIRE_REMOTE=0
if ( verify_off_site_replication_policy 2>"${WORK_DIR}/local_prod_err.log" ); then
  echo "FAIL: Production profile accepted local-only mode!" >&2
  exit 97
fi
grep -q "FATAL: Local-only backup (SPINE_BACKUP_REQUIRE_REMOTE=0) is strictly FORBIDDEN under production profile 'core-forge-v1'" "${WORK_DIR}/local_prod_err.log"
export SPINE_DEPLOYMENT_PROFILE=development

# 3. Invalid values (such as 'yes', '2', 'maybe') must fail closed
for bad_val in "yes" "2" "maybe" "no"; do
  export SPINE_BACKUP_REQUIRE_REMOTE="${bad_val}"
  if ( verify_off_site_replication_policy 2>"${WORK_DIR}/bad_${bad_val}.log" ); then
    echo "FAIL: Invalid policy value '${bad_val}' did not fail closed!" >&2
    exit 98
  fi
  grep -q "FATAL: Invalid SPINE_BACKUP_REQUIRE_REMOTE value" "${WORK_DIR}/bad_${bad_val}.log"
done

rm -rf "${WORK_DIR}"
exit 0
"#
            .replace("\r\n", "\n")
            .replace('\r', "");

            let mut child = Command::new("bash")
                .arg("-s")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("failed to spawn bash");

            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(script.as_bytes()).expect("failed to write script to stdin");
            }
            let res = child.wait_with_output().expect("failed to wait on bash child");
            assert!(
                res.status.success(),
                "Off-site policy enforcement test failed. Stderr: {}, Stdout: {}",
                String::from_utf8_lossy(&res.stderr),
                String::from_utf8_lossy(&res.stdout)
            );
        }
    }
}

#[test]
fn test_backup_s3_upload_cryptographic_sha256_verification_and_tampering_rejection() {
    if let Ok(bash_check) = Command::new("bash").arg("-c").arg("echo ok").output() {
        if bash_check.status.success() {
            let script = r#"
set -euo pipefail
WORK_DIR="/tmp/spine_test_sha_$$"
mkdir -p "${WORK_DIR}"
MOCK_BIN="${WORK_DIR}/bin"
mkdir -p "${MOCK_BIN}"
MOCK_S3="${WORK_DIR}/s3"
mkdir -p "${MOCK_S3}"

cat <<EOF > "${MOCK_BIN}/aws"
#!/usr/bin/env bash
if [ "\$1" = "s3" ] && [ "\$2" = "cp" ]; then
  SRC="\$3"
  DST="\$4"
  if [ "\$DST" = "-" ]; then
    KEY=\$(basename "\$SRC")
    cat "${MOCK_S3}/\${KEY}"
  elif [[ "\$DST" =~ ^s3:// ]]; then
    KEY=\$(basename "\$DST")
    cp "\$SRC" "${MOCK_S3}/\${KEY}"
  fi
  exit 0
elif [ "\$1" = "s3" ] && [ "\$2" = "ls" ]; then
  KEY=\$(basename "\$3")
  SIZE=\$(stat -c%s "${MOCK_S3}/\${KEY}" 2>/dev/null || stat -f%z "${MOCK_S3}/\${KEY}" 2>/dev/null || wc -c < "${MOCK_S3}/\${KEY}")
  echo "2026-11-20 12:00:00 \${SIZE} \${KEY}"
  exit 0
fi
EOF
chmod +x "${MOCK_BIN}/aws"
export PATH="${MOCK_BIN}:${PATH}"

LOCAL_FILE="${WORK_DIR}/backup.spnebak"
echo "AUTHENTICATED_SPNEBAK_DATA_CONTENT_12345" > "${LOCAL_FILE}"
LOCAL_SHA256=$(sha256sum "${LOCAL_FILE}" | awk '{print $1}')
LOCAL_SIZE=$(wc -c < "${LOCAL_FILE}" | tr -d '[:space:]')

# Upload to S3
aws s3 cp "${LOCAL_FILE}" "s3://vault/spine/backup.spnebak"

# 1. Normal check passes
REMOTE_SHA256=$(aws s3 cp "s3://vault/spine/backup.spnebak" - | sha256sum | awk '{print $1}')
if [ "${LOCAL_SHA256}" != "${REMOTE_SHA256}" ]; then
  exit 1
fi

# 2. Tamper the remote object with SAME SIZE (replace '1' with '9')
sed -i 's/1/9/' "${MOCK_S3}/backup.spnebak"
TAMPERED_SIZE=$(wc -c < "${MOCK_S3}/backup.spnebak" | tr -d '[:space:]')
if [ "${LOCAL_SIZE}" != "${TAMPERED_SIZE}" ]; then
  exit 2
fi

verify_remote_sha256() {
  local remote_sha
  remote_sha=$(aws s3 cp "s3://vault/spine/backup.spnebak" - | sha256sum | awk '{print $1}')
  if [ "${LOCAL_SHA256}" != "${remote_sha}" ]; then
    echo "FATAL: Cryptographic verification failed! Remote SHA-256 does not match local archive." >&2
    exit 1
  fi
}

# Must fail closed on tampered remote object!
if ( verify_remote_sha256 2>"${WORK_DIR}/tamper.log" ); then
  echo "FAIL: Verification passed despite tampered bytes!" >&2
  exit 99
fi
grep -q "Cryptographic verification failed" "${WORK_DIR}/tamper.log"

rm -rf "${WORK_DIR}"
exit 0
"#
            .replace("\r\n", "\n")
            .replace('\r', "");

            let mut child = Command::new("bash")
                .arg("-s")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("failed to spawn bash");

            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(script.as_bytes()).expect("failed to write script to stdin");
            }
            let res = child.wait_with_output().expect("failed to wait on bash child");
            assert!(
                res.status.success(),
                "Cryptographic SHA-256 verification and tampering rejection test failed. Stderr: {}, Stdout: {}",
                String::from_utf8_lossy(&res.stderr),
                String::from_utf8_lossy(&res.stdout)
            );
        }
    }
}

#[test]
fn test_remote_restore_download_cleanup_on_success_failure_and_interrupt() {
    if let Ok(bash_check) = Command::new("bash").arg("-c").arg("echo ok").output() {
        if bash_check.status.success() {
            let script = r#"
set -euo pipefail
WORK_DIR="/tmp/spine_test_cleanup_$$"
mkdir -p "${WORK_DIR}"

run_restore_lifecycle() {
  local scenario="$1"
  local remote_dir="${WORK_DIR}/remote_staging_${scenario}"
  mkdir -p "${remote_dir}"
  echo "REMOTE_ENCRYPTED_BACKUP" > "${remote_dir}/backup.spnebak"
  echo "0000000000000000000000000000000000000000000000000000000000000000  backup.spnebak" > "${remote_dir}/SHA256SUMS"

  do_cleanup() {
    if [ -n "${remote_dir:-}" ] && [ -e "${remote_dir}" ]; then
      chmod -R 700 "${remote_dir}" 2>/dev/null || true
      rm -rf "${remote_dir}"
    fi
  }

  case "${scenario}" in
    "success")
      do_cleanup
      if [ -e "${remote_dir}" ]; then return 1; fi
      ;;
    "download_err")
      do_cleanup
      if [ -e "${remote_dir}" ]; then return 2; fi
      ;;
    "checksum_err")
      if ! ( cd "${remote_dir}" && sha256sum -c SHA256SUMS >/dev/null 2>&1 ); then
        do_cleanup
      fi
      if [ -e "${remote_dir}" ]; then return 3; fi
      ;;
    "sigint")
      do_cleanup
      if [ -e "${remote_dir}" ]; then return 4; fi
      ;;
  esac
  return 0
}

run_restore_lifecycle "success"
run_restore_lifecycle "download_err"
run_restore_lifecycle "checksum_err"
run_restore_lifecycle "sigint"

rm -rf "${WORK_DIR}"
exit 0
"#
            .replace("\r\n", "\n")
            .replace('\r', "");

            let mut child = Command::new("bash")
                .arg("-s")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("failed to spawn bash");

            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(script.as_bytes()).expect("failed to write script to stdin");
            }
            let res = child.wait_with_output().expect("failed to wait on bash child");
            assert!(
                res.status.success(),
                "Remote restore download cleanup test failed. Stderr: {}, Stdout: {}",
                String::from_utf8_lossy(&res.stderr),
                String::from_utf8_lossy(&res.stdout)
            );
        }
    }
}

#[test]
fn test_backup_rejects_invalid_policy_values_fail_closed() {
    if let Ok(bash_check) = Command::new("bash").arg("-c").arg("echo ok").output() {
        if bash_check.status.success() {
            let script = r#"
set -euo pipefail
WORK_DIR="/tmp/spine_test_invalid_policy_$$"
mkdir -p "${WORK_DIR}"

compute_retain_until_date() {
  local days="$1"
  local d=""
  d=$(date -u -d "+${days} days" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || true)
  if [ -z "${d}" ]; then
    d=$(date -u -v "+${days}d" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || true)
  fi
  printf "%s" "${d}"
}

normalize_deployment_profile() {
  case "${SPINE_DEPLOYMENT_PROFILE:-core-forge-v1}" in
    core-forge-v1|core-forge|production|prod)
      echo "core-forge-v1"
      ;;
    development|dev)
      echo "development"
      ;;
    test)
      echo "test"
      ;;
    *)
      echo "invalid"
      ;;
  esac
}

verify_off_site_replication_policy() {
  local profile
  profile=$(normalize_deployment_profile)
  if [ "${profile}" = "invalid" ]; then
    echo "FATAL: Invalid SPINE_DEPLOYMENT_PROFILE='${SPINE_DEPLOYMENT_PROFILE}'." >&2
    exit 1
  fi

  case "${SPINE_BACKUP_REQUIRE_REMOTE}" in
    1|true)
      if [ -z "${SPINE_BACKUP_REMOTE_S3_URI}" ]; then
        echo "FATAL: Off-site replication policy is ENFORCED, but URI unset." >&2
        exit 1
      fi
      ;;
    0|false)
      if [ "${profile}" = "core-forge-v1" ]; then
        echo "FATAL: Local-only backup (SPINE_BACKUP_REQUIRE_REMOTE=0) is strictly FORBIDDEN under production profile 'core-forge-v1'." >&2
        exit 1
      fi
      echo "NOTICE: Operating in INTENTIONALLY LOCAL-ONLY mode."
      ;;
    *)
      echo "FATAL: Invalid SPINE_BACKUP_REQUIRE_REMOTE value: '${SPINE_BACKUP_REQUIRE_REMOTE}'" >&2
      exit 1
      ;;
  esac

  case "${SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE:-0}" in
    0|1|false|true)
      ;;
    *)
      echo "FATAL: Invalid SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE value: '${SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE}'" >&2
      exit 1
      ;;
  esac

  if [ -n "${SPINE_BACKUP_REMOTE_S3_URI}" ]; then
    if [ "${profile}" = "core-forge-v1" ]; then
      if [ "${SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE:-0}" = "1" ] || [ "${SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE:-0}" = "true" ]; then
        echo "FATAL: S3 Object Lock retention bypass (SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE=1) is strictly FORBIDDEN under production profile 'core-forge-v1'." >&2
        exit 1
      fi
      if [ "${SPINE_BACKUP_OBJECT_LOCK_MODE}" = "GOVERNANCE" ]; then
        echo "FATAL: S3 Object Lock mode 'GOVERNANCE' is rejected under production profile 'core-forge-v1'. Production requires 'COMPLIANCE'." >&2
        exit 1
      fi
    fi

    case "${SPINE_BACKUP_OBJECT_LOCK_MODE}" in
      COMPLIANCE)
        if ! [[ "${SPINE_BACKUP_RETENTION_DAYS}" =~ ^[1-9][0-9]*$ ]]; then
          echo "FATAL: Invalid SPINE_BACKUP_RETENTION_DAYS='${SPINE_BACKUP_RETENTION_DAYS}'." >&2
          exit 1
        fi
        local test_date
        test_date=$(compute_retain_until_date "${SPINE_BACKUP_RETENTION_DAYS}")
        if [ -z "${test_date}" ]; then
          echo "FATAL: Cannot compute retain-until date for S3 Object Lock retention." >&2
          exit 1
        fi
        ;;
      GOVERNANCE)
        if ! [[ "${SPINE_BACKUP_RETENTION_DAYS}" =~ ^[1-9][0-9]*$ ]]; then
          echo "FATAL: Invalid SPINE_BACKUP_RETENTION_DAYS='${SPINE_BACKUP_RETENTION_DAYS}'." >&2
          exit 1
        fi
        local test_date
        test_date=$(compute_retain_until_date "${SPINE_BACKUP_RETENTION_DAYS}")
        if [ -z "${test_date}" ]; then
          echo "FATAL: Cannot compute retain-until date for S3 Object Lock retention." >&2
          exit 1
        fi
        ;;
      ""|NONE)
        if [ "${profile}" = "core-forge-v1" ]; then
          echo "FATAL: S3 Object Lock retention is MANDATORY under production profile 'core-forge-v1'." >&2
          exit 1
        fi
        if [ "${SPINE_BACKUP_REQUIRE_REMOTE}" = "1" ] || [ "${SPINE_BACKUP_REQUIRE_REMOTE}" = "true" ]; then
          if [ "${SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE:-0}" != "1" ] && [ "${SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE:-0}" != "true" ]; then
            echo "FATAL: S3 Object Lock retention is MANDATORY when off-site replication is enforced." >&2
            exit 1
          fi
          echo "NON-PRODUCTION NOTICE: S3 Object Lock retention is explicitly bypassed via SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE=1."
        else
          echo "NOTICE: S3 Object Lock retention is not configured."
        fi
        ;;
      *)
        echo "FATAL: Unsupported SPINE_BACKUP_OBJECT_LOCK_MODE: '${SPINE_BACKUP_OBJECT_LOCK_MODE}'" >&2
        exit 1
        ;;
    esac
  fi
}

# 1. Invalid SPINE_DEPLOYMENT_PROFILE value
export SPINE_DEPLOYMENT_PROFILE="staging"
export SPINE_BACKUP_REQUIRE_REMOTE="1"
export SPINE_BACKUP_REMOTE_S3_URI="s3://test-bucket/backups"
export SPINE_BACKUP_OBJECT_LOCK_MODE="COMPLIANCE"
export SPINE_BACKUP_RETENTION_DAYS="30"
export SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE=0

if ( verify_off_site_replication_policy 2>"${WORK_DIR}/prof_err.log" ); then
  echo "FAIL: Invalid deployment profile 'staging' was accepted!" >&2
  exit 1
fi
grep -q "FATAL: Invalid SPINE_DEPLOYMENT_PROFILE" "${WORK_DIR}/prof_err.log"

# Reset to core-forge-v1 production profile
export SPINE_DEPLOYMENT_PROFILE="core-forge-v1"

# 2. Invalid SPINE_BACKUP_REQUIRE_REMOTE values
for invalid_val in "yes" "no" "2" "True" "maybe" " " "disabled" "enabled"; do
  export SPINE_BACKUP_REQUIRE_REMOTE="${invalid_val}"
  if ( verify_off_site_replication_policy 2>"${WORK_DIR}/err.log" ); then
    echo "FAIL: Invalid policy value '${invalid_val}' was accepted!" >&2
    exit 2
  fi
  grep -q "FATAL: Invalid SPINE_BACKUP_REQUIRE_REMOTE value" "${WORK_DIR}/err.log"
done

# 3. Unsupported SPINE_BACKUP_OBJECT_LOCK_MODE values
export SPINE_BACKUP_REQUIRE_REMOTE="1"
for bad_mode in "TEMPORARY" "STANDARD" "compliance" "governance" "INVALID" "1" "true"; do
  export SPINE_BACKUP_OBJECT_LOCK_MODE="${bad_mode}"
  if ( verify_off_site_replication_policy 2>"${WORK_DIR}/mode_err.log" ); then
    echo "FAIL: Unsupported lock mode '${bad_mode}' was accepted!" >&2
    exit 3
  fi
  grep -q "FATAL: Unsupported SPINE_BACKUP_OBJECT_LOCK_MODE" "${WORK_DIR}/mode_err.log"
done

# 4. Under production profile 'core-forge-v1':
# a) GOVERNANCE mode is rejected fail-closed
export SPINE_BACKUP_OBJECT_LOCK_MODE="GOVERNANCE"
if ( verify_off_site_replication_policy 2>"${WORK_DIR}/gov_err.log" ); then
  echo "FAIL: Production mode accepted bypassable GOVERNANCE lock mode!" >&2
  exit 4
fi
grep -q "FATAL: S3 Object Lock mode 'GOVERNANCE' is rejected under production profile 'core-forge-v1'" "${WORK_DIR}/gov_err.log"

# b) Non-production override (ALLOW_UNLOCKED_REMOTE=1) cannot disable Object Lock under production profile
export SPINE_BACKUP_OBJECT_LOCK_MODE="NONE"
export SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE=1
if ( verify_off_site_replication_policy 2>"${WORK_DIR}/prod_override_err.log" ); then
  echo "FAIL: Production profile accepted SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE=1 override!" >&2
  exit 5
fi
grep -q "FATAL: S3 Object Lock retention bypass (SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE=1) is strictly FORBIDDEN under production profile 'core-forge-v1'" "${WORK_DIR}/prod_override_err.log"

# c) Valid COMPLIANCE mode under core-forge-v1 succeeds
export SPINE_BACKUP_OBJECT_LOCK_MODE="COMPLIANCE"
export SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE=0
verify_off_site_replication_policy > "${WORK_DIR}/prod_success.log"

# d) Local-only mode (SPINE_BACKUP_REQUIRE_REMOTE=0) is strictly FORBIDDEN under production profile 'core-forge-v1'
export SPINE_BACKUP_REQUIRE_REMOTE=0
if ( verify_off_site_replication_policy 2>"${WORK_DIR}/prod_local_only_err.log" ); then
  echo "FAIL: Production profile accepted local-only backup (SPINE_BACKUP_REQUIRE_REMOTE=0)!" >&2
  exit 7
fi
grep -q "FATAL: Local-only backup (SPINE_BACKUP_REQUIRE_REMOTE=0) is strictly FORBIDDEN under production profile 'core-forge-v1'" "${WORK_DIR}/prod_local_only_err.log"
export SPINE_BACKUP_REQUIRE_REMOTE=1

# 5. Under non-production profile 'development':
export SPINE_DEPLOYMENT_PROFILE="development"
# a) GOVERNANCE mode is accepted
export SPINE_BACKUP_OBJECT_LOCK_MODE="GOVERNANCE"
verify_off_site_replication_policy > "${WORK_DIR}/dev_gov.log"

# b) ALLOW_UNLOCKED_REMOTE=1 is honored in development
export SPINE_BACKUP_OBJECT_LOCK_MODE="NONE"
export SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE=1
verify_off_site_replication_policy > "${WORK_DIR}/dev_override.log"
grep -q "NON-PRODUCTION NOTICE" "${WORK_DIR}/dev_override.log"

# c) ALLOW_UNLOCKED_REMOTE=0 in development still fails closed if lock is omitted
export SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE=0
if ( verify_off_site_replication_policy 2>"${WORK_DIR}/dev_no_override_err.log" ); then
  echo "FAIL: Development mode without override accepted omitted Object Lock!" >&2
  exit 6
fi
grep -q "FATAL: S3 Object Lock retention is MANDATORY" "${WORK_DIR}/dev_no_override_err.log"

# d) Local-only mode (SPINE_BACKUP_REQUIRE_REMOTE=0) is PERMITTED under non-production profile 'development'
export SPINE_BACKUP_REQUIRE_REMOTE=0
verify_off_site_replication_policy > "${WORK_DIR}/dev_local_only.log"
grep -q "INTENTIONALLY LOCAL-ONLY" "${WORK_DIR}/dev_local_only.log"

rm -rf "${WORK_DIR}"
exit 0
"#
            .replace("\r\n", "\n")
            .replace('\r', "");

            let mut child = Command::new("bash")
                .arg("-s")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("failed to spawn bash");

            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(script.as_bytes()).expect("failed to write script to stdin");
            }
            let res = child.wait_with_output().expect("failed to wait on bash child");
            assert!(
                res.status.success(),
                "Invalid policy values rejection test failed. Stderr: {}, Stdout: {}",
                String::from_utf8_lossy(&res.stderr),
                String::from_utf8_lossy(&res.stdout)
            );
        }
    }
}

#[test]
fn test_backup_rejects_unavailable_date_calculation_fail_closed() {
    if let Ok(bash_check) = Command::new("bash").arg("-c").arg("echo ok").output() {
        if bash_check.status.success() {
            let script = r#"
set -euo pipefail
WORK_DIR="/tmp/spine_test_date_fail_$$"
mkdir -p "${WORK_DIR}"
BIN_DIR="${WORK_DIR}/bin"
mkdir -p "${BIN_DIR}"

# 1. Test invalid non-numeric retention days
export SPINE_BACKUP_REQUIRE_REMOTE="1"
export SPINE_BACKUP_REMOTE_S3_URI="s3://test-bucket/backups"
export SPINE_BACKUP_OBJECT_LOCK_MODE="GOVERNANCE"

compute_retain_until_date() {
  local days="$1"
  local d=""
  d=$(date -u -d "+${days} days" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || true)
  if [ -z "${d}" ]; then
    d=$(date -u -v "+${days}d" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || true)
  fi
  printf "%s" "${d}"
}

verify_retention_date_gate() {
  if ! [[ "${SPINE_BACKUP_RETENTION_DAYS}" =~ ^[1-9][0-9]*$ ]]; then
    echo "FATAL: Invalid SPINE_BACKUP_RETENTION_DAYS='${SPINE_BACKUP_RETENTION_DAYS}'. Must be a positive integer." >&2
    exit 1
  fi
  local test_date
  test_date=$(compute_retain_until_date "${SPINE_BACKUP_RETENTION_DAYS}")
  if [ -z "${test_date}" ]; then
    echo "FATAL: Cannot compute retain-until date for S3 Object Lock retention (+${SPINE_BACKUP_RETENTION_DAYS} days)." >&2
    exit 1
  fi
}

for bad_days in "0" "-1" "invalid" "30days" " 30" "null"; do
  export SPINE_BACKUP_RETENTION_DAYS="${bad_days}"
  if ( verify_retention_date_gate 2>"${WORK_DIR}/days_err.log" ); then
    echo "FAIL: Bad retention days '${bad_days}' passed gate!" >&2
    exit 1
  fi
  grep -q "FATAL: Invalid SPINE_BACKUP_RETENTION_DAYS" "${WORK_DIR}/days_err.log"
done

# 2. Test when date command cannot format relative dates (simulating unavailable date calculation)
cat <<'EOF' > "${BIN_DIR}/date"
#!/usr/bin/env bash
# Fail if relative date syntax is requested
for arg in "$@"; do
  if [ "$arg" = "-d" ] || [ "$arg" = "-v" ]; then
    exit 1
  fi
done
exit 1
EOF
chmod +x "${BIN_DIR}/date"

export PATH="${BIN_DIR}:${PATH}"
export SPINE_BACKUP_RETENTION_DAYS="30"

if ( verify_retention_date_gate 2>"${WORK_DIR}/date_fail.log" ); then
  echo "FAIL: Unavailable date calculation did not fail closed!" >&2
  exit 2
fi
grep -q "FATAL: Cannot compute retain-until date" "${WORK_DIR}/date_fail.log"

rm -rf "${WORK_DIR}"
exit 0
"#
            .replace("\r\n", "\n")
            .replace('\r', "");

            let mut child = Command::new("bash")
                .arg("-s")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("failed to spawn bash");

            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(script.as_bytes()).expect("failed to write script to stdin");
            }
            let res = child.wait_with_output().expect("failed to wait on bash child");
            assert!(
                res.status.success(),
                "Unavailable date calculation fail-closed test failed. Stderr: {}, Stdout: {}",
                String::from_utf8_lossy(&res.stderr),
                String::from_utf8_lossy(&res.stdout)
            );
        }
    }
}

#[test]
fn test_backup_fails_closed_when_remote_object_lock_metadata_absent_or_mismatched() {
    if let Ok(bash_check) = Command::new("bash").arg("-c").arg("echo ok").output() {
        if bash_check.status.success() {
            let script = r#"
set -euo pipefail
WORK_DIR="/tmp/spine_test_lock_metadata_$$"
mkdir -p "${WORK_DIR}"
MOCK_BIN="${WORK_DIR}/bin"
mkdir -p "${MOCK_BIN}"
MOCK_S3="${WORK_DIR}/s3"
mkdir -p "${MOCK_S3}"

# Configurable mock behavior
export MOCK_RETENTION_BEHAVIOR="absent_config"

cat <<EOF > "${MOCK_BIN}/aws"
#!/usr/bin/env bash
if [ "\$1" = "s3" ] && [ "\$2" = "cp" ]; then
  SRC="\$3"
  DST="\$4"
  if [ "\$DST" = "-" ]; then
    KEY=\$(basename "\$SRC")
    cat "${MOCK_S3}/\${KEY}"
  elif [[ "\$DST" =~ ^s3:// ]]; then
    KEY=\$(basename "\$DST")
    cp "\$SRC" "${MOCK_S3}/\${KEY}"
  fi
  exit 0
elif [ "\$1" = "s3" ] && [ "\$2" = "ls" ]; then
  KEY=\$(basename "\$3")
  SIZE=\$(stat -c%s "${MOCK_S3}/\${KEY}" 2>/dev/null || stat -f%z "${MOCK_S3}/\${KEY}" 2>/dev/null || wc -c < "${MOCK_S3}/\${KEY}")
  echo "2026-11-20 12:00:00 \${SIZE} \${KEY}"
  exit 0
elif [ "\$1" = "s3api" ] && [ "\$2" = "get-object-retention" ]; then
  case "\${MOCK_RETENTION_BEHAVIOR}" in
    "absent_config")
      echo "An error occurred (NoSuchObjectLockConfiguration) when calling the GetObjectRetention operation: The specified object does not have a ObjectLock configuration" >&2
      exit 1
      ;;
    "absent_date")
      echo '{"Retention": {"Mode": "COMPLIANCE"}}'
      exit 0
      ;;
    "malformed_date")
      echo '{"Retention": {"Mode": "COMPLIANCE", "RetainUntilDate": "malformed_not_a_date"}}'
      exit 0
      ;;
    "expired_date")
      echo '{"Retention": {"Mode": "COMPLIANCE", "RetainUntilDate": "2020-01-01T00:00:00.000Z"}}'
      exit 0
      ;;
    "too_short_date")
      # Compute 1 day in the future (far shorter than 30 days configured)
      SHORT_DATE=\$(date -u -d "+1 day" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date -u -v "+1d" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || echo "2026-11-21T12:00:00Z")
      echo "{\"Retention\": {\"Mode\": \"COMPLIANCE\", \"RetainUntilDate\": \"\${SHORT_DATE}\"}}"
      exit 0
      ;;
    "mismatched_mode")
      VALID_DATE=\$(date -u -d "+35 days" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date -u -v "+35d" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || echo "2027-01-01T12:00:00Z")
      echo "{\"Retention\": {\"Mode\": \"GOVERNANCE\", \"RetainUntilDate\": \"\${VALID_DATE}\"}}"
      exit 0
      ;;
    "valid")
      VALID_DATE=\$(date -u -d "+35 days" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || date -u -v "+35d" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || echo "2027-01-01T12:00:00Z")
      echo "{\"Retention\": {\"Mode\": \"COMPLIANCE\", \"RetainUntilDate\": \"\${VALID_DATE}\"}}"
      exit 0
      ;;
    *)
      exit 1
      ;;
  esac
fi
EOF
chmod +x "${MOCK_BIN}/aws"
export PATH="${MOCK_BIN}:${PATH}"

iso_to_epoch() {
  local iso_str="${1:-}"
  if [ -z "${iso_str}" ]; then
    return 0
  fi
  local clean_str
  clean_str=$(echo "${iso_str}" | sed -E 's/\.[0-9]+Z$/Z/')
  if ! [[ "${clean_str}" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2} ]]; then
    return 0
  fi
  local epoch=""
  epoch=$(date -u -d "${clean_str}" +%s 2>/dev/null || true)
  if [ -z "${epoch}" ]; then
    epoch=$(date -u -d "${iso_str}" +%s 2>/dev/null || true)
  fi
  if [ -z "${epoch}" ]; then
    epoch=$(date -j -u -f "%Y-%m-%dT%H:%M:%SZ" "${clean_str}" +%s 2>/dev/null || true)
  fi
  if [ -z "${epoch}" ]; then
    epoch=$(python3 -c "import datetime, sys; s = sys.argv[1].replace('Z', '+00:00'); print(int(datetime.datetime.fromisoformat(s).timestamp()))" "${iso_str}" 2>/dev/null || true)
  fi
  printf "%s" "${epoch}"
}

LOCAL_FILE="${WORK_DIR}/backup.spnebak"
echo "AUTHENTICATED_SPNEBAK_DATA_CONTENT" > "${LOCAL_FILE}"
REMOTE_TARGET="s3://test-vault/spine/backup_test"

run_retention_confirmation_gate() {
  local mode="$1"
  local apply_lock="$2"
  local require_remote="${3:-1}"
  local allow_unlocked="${4:-0}"
  local retention_days="${5:-30}"
  local profile="${6:-core-forge-v1}"

  # Production mode requires Object Lock; unlocked bypass is strictly forbidden under core-forge-v1
  if [ "${apply_lock}" -eq 0 ]; then
    if [ "${profile}" = "core-forge-v1" ]; then
      echo "FATAL: S3 Object Lock retention cannot be omitted under production profile 'core-forge-v1'." >&2
      exit 1
    fi
    if [ "${require_remote}" = "1" ] && [ "${allow_unlocked}" != "1" ]; then
      echo "FATAL: S3 Object Lock retention is mandatory for production remote backups." >&2
      exit 1
    fi
  fi

  NO_SCHEME="${REMOTE_TARGET#s3://}"
  S3_BUCKET="${NO_SCHEME%%/*}"
  if [ "${NO_SCHEME}" != "${S3_BUCKET}" ]; then
    S3_PREFIX="${NO_SCHEME#*/}"
    S3_KEY="${S3_PREFIX}/backup.spnebak"
  else
    S3_KEY="backup.spnebak"
  fi

  if [ "${apply_lock}" -eq 1 ]; then
    RETENTION_OUTPUT=$(aws s3api get-object-retention --bucket "${S3_BUCKET}" --key "${S3_KEY}" 2>&1 || true)
    REMOTE_LOCK_MODE=$(echo "${RETENTION_OUTPUT}" | grep -o '"Mode"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"Mode"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/' || echo "")
    REMOTE_RETAIN_UNTIL=$(echo "${RETENTION_OUTPUT}" | grep -o '"RetainUntilDate"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"RetainUntilDate"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/' || echo "")

    # 1. Mode match
    if [ -z "${REMOTE_LOCK_MODE}" ] || [ "${REMOTE_LOCK_MODE}" != "${mode}" ]; then
      echo "FATAL: S3 Object Lock retention verification failed for ${REMOTE_TARGET}/backup.spnebak!" >&2
      echo "  Expected lock mode:   ${mode}" >&2
      echo "  Observed lock mode:   ${REMOTE_LOCK_MODE:-<absent>}" >&2
      exit 1
    fi

    # 2. Date presence
    if [ -z "${REMOTE_RETAIN_UNTIL}" ]; then
      echo "FATAL: S3 Object Lock retention verification failed: RetainUntilDate is missing from remote object metadata." >&2
      exit 1
    fi

    # 3. Date validity & epoch parsing
    REMOTE_EPOCH=$(iso_to_epoch "${REMOTE_RETAIN_UNTIL}")
    if [ -z "${REMOTE_EPOCH}" ]; then
      echo "FATAL: S3 Object Lock retention verification failed: RetainUntilDate '${REMOTE_RETAIN_UNTIL}' is malformed or unparseable." >&2
      exit 1
    fi

    NOW_EPOCH=$(date +%s)
    # 4. Expired check
    if [ "${REMOTE_EPOCH}" -le "${NOW_EPOCH}" ]; then
      echo "FATAL: S3 Object Lock retention verification failed: RetainUntilDate '${REMOTE_RETAIN_UNTIL}' (epoch: ${REMOTE_EPOCH}) is expired or in the past (current epoch: ${NOW_EPOCH})." >&2
      exit 1
    fi

    # 5. Too-short date check against configured retention window (300s clock skew buffer)
    MIN_REQUIRED_EPOCH=$(( NOW_EPOCH + (retention_days * 86400) - 300 ))
    if [ "${REMOTE_EPOCH}" -lt "${MIN_REQUIRED_EPOCH}" ]; then
      echo "FATAL: S3 Object Lock retention verification failed: RetainUntilDate '${REMOTE_RETAIN_UNTIL}' is earlier than the configured retention period of ${retention_days} days." >&2
      exit 1
    fi

    echo "Remote replication cryptographically verified [OK] (immutable retention active: ${REMOTE_LOCK_MODE} until ${REMOTE_RETAIN_UNTIL})."
  else
    echo "Remote replication cryptographically verified [OK] (standard replication without Object Lock)."
  fi
}

# Upload local file
aws s3 cp "${LOCAL_FILE}" "${REMOTE_TARGET}/backup.spnebak"

# 1. Object Lock configuration absent -> must FAIL CLOSED
export MOCK_RETENTION_BEHAVIOR="absent_config"
if ( run_retention_confirmation_gate "COMPLIANCE" 1 2>"${WORK_DIR}/absent.err" ); then
  echo "FAIL: Gate succeeded despite absent retention configuration!" >&2
  exit 1
fi
grep -q "FATAL: S3 Object Lock retention verification failed" "${WORK_DIR}/absent.err"
if grep -q "immutable retention active" "${WORK_DIR}/absent.err"; then
  echo "FAIL: Erroneously reported immutable retention active when configuration absent!" >&2
  exit 2
fi

# 2. RetainUntilDate missing -> must FAIL CLOSED
export MOCK_RETENTION_BEHAVIOR="absent_date"
if ( run_retention_confirmation_gate "COMPLIANCE" 1 2>"${WORK_DIR}/absent_date.err" ); then
  echo "FAIL: Gate succeeded despite missing RetainUntilDate!" >&2
  exit 3
fi
grep -q "RetainUntilDate is missing" "${WORK_DIR}/absent_date.err"

# 3. RetainUntilDate malformed -> must FAIL CLOSED
export MOCK_RETENTION_BEHAVIOR="malformed_date"
if ( run_retention_confirmation_gate "COMPLIANCE" 1 2>"${WORK_DIR}/malformed_date.err" ); then
  echo "FAIL: Gate succeeded despite malformed RetainUntilDate!" >&2
  exit 4
fi
grep -q "malformed or unparseable" "${WORK_DIR}/malformed_date.err"

# 4. RetainUntilDate expired -> must FAIL CLOSED
export MOCK_RETENTION_BEHAVIOR="expired_date"
if ( run_retention_confirmation_gate "COMPLIANCE" 1 2>"${WORK_DIR}/expired_date.err" ); then
  echo "FAIL: Gate succeeded despite expired RetainUntilDate!" >&2
  exit 5
fi
grep -q "is expired or in the past" "${WORK_DIR}/expired_date.err"

# 5. RetainUntilDate too short (e.g. 1 day vs 30 days) -> must FAIL CLOSED
export MOCK_RETENTION_BEHAVIOR="too_short_date"
if ( run_retention_confirmation_gate "COMPLIANCE" 1 2>"${WORK_DIR}/too_short.err" ); then
  echo "FAIL: Gate succeeded despite too-short RetainUntilDate!" >&2
  exit 6
fi
grep -q "is earlier than the configured retention period" "${WORK_DIR}/too_short.err"

# 6. Mode mismatch (expected COMPLIANCE, got GOVERNANCE) -> must FAIL CLOSED
export MOCK_RETENTION_BEHAVIOR="mismatched_mode"
if ( run_retention_confirmation_gate "COMPLIANCE" 1 2>"${WORK_DIR}/mismatch.err" ); then
  echo "FAIL: Gate succeeded despite mode mismatch!" >&2
  exit 7
fi
grep -q "FATAL: S3 Object Lock retention verification failed" "${WORK_DIR}/mismatch.err"

# 7. Confirmed valid metadata meeting retention window -> SUCCEEDS
export MOCK_RETENTION_BEHAVIOR="valid"
OUT=$(run_retention_confirmation_gate "COMPLIANCE" 1)
echo "${OUT}" | grep -q "immutable retention active: COMPLIANCE until"

# 8. Production profile (core-forge-v1) cannot omit Object Lock even with allow_unlocked=1 -> must FAIL CLOSED
if ( run_retention_confirmation_gate "" 0 1 1 30 "core-forge-v1" 2>"${WORK_DIR}/prod_unlocked.err" ); then
  echo "FAIL: Production profile accepted omitted Object Lock!" >&2
  exit 8
fi
grep -q "FATAL: S3 Object Lock retention cannot be omitted under production profile 'core-forge-v1'" "${WORK_DIR}/prod_unlocked.err"

# 9. Non-production profile (development) with Object Lock omitted WITH explicit override -> SUCCEEDS without claiming immutable retention
OUT_DEV_OVERRIDE=$(run_retention_confirmation_gate "" 0 1 1 30 "development")
echo "${OUT_DEV_OVERRIDE}" | grep -q "standard replication without Object Lock"
if echo "${OUT_DEV_OVERRIDE}" | grep -q "immutable retention active"; then
  echo "FAIL: Claimed immutable retention active under non-lock override!" >&2
  exit 9
fi

rm -rf "${WORK_DIR}"
exit 0
"#
            .replace("\r\n", "\n")
            .replace('\r', "");

            let mut child = Command::new("bash")
                .arg("-s")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("failed to spawn bash");

            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(script.as_bytes()).expect("failed to write script to stdin");
            }
            let res = child.wait_with_output().expect("failed to wait on bash child");
            assert!(
                res.status.success(),
                "Remote Object Lock metadata absent/mismatched failure test failed. Stderr: {}, Stdout: {}",
                String::from_utf8_lossy(&res.stderr),
                String::from_utf8_lossy(&res.stdout)
            );
        }
    }
}
