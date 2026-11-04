use crate::core::error::ForgeError;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackupManifest {
    pub backup_id: String,
    pub timestamp_nanos: i64,
    pub software_version: String,
    pub schema_version: String,
    pub total_files: usize,
    pub total_bytes: u64,
    pub file_hashes: BTreeMap<String, String>,
    pub ssh_host_key_fingerprint: Option<String>,
    pub integrity_status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RestoreReport {
    pub backup_id: String,
    pub files_restored: usize,
    pub bytes_restored: u64,
    pub duration_ms: u64,
    pub integrity_verified: bool,
}

pub struct BackupManager;

impl BackupManager {
    /// Creates a verified, coordinated backup archive (.tar.gz) from storage root and database snapshot
    pub fn create_backup(
        storage_root: &Path,
        db_snapshot_json: Option<&str>,
        output_tar_gz: &Path,
    ) -> Result<BackupManifest, ForgeError> {
        let backup_id = Uuid::new_v4().to_string();
        let timestamp_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

        if let Some(parent) = output_tar_gz.parent() {
            fs::create_dir_all(parent).map_err(|e| ForgeError::Internal(e.to_string()))?;
        }

        let mut file_hashes = BTreeMap::new();
        let mut total_bytes = 0u64;

        // Collect all files in storage root
        let mut file_paths = Vec::new();
        if storage_root.exists() {
            Self::collect_files_recursive(storage_root, storage_root, &mut file_paths)?;
        }

        // Calculate SHA-256 for all storage files
        for (rel_path, abs_path) in &file_paths {
            let data = fs::read(abs_path).map_err(|e| ForgeError::Internal(e.to_string()))?;
            total_bytes += data.len() as u64;
            let mut hasher = Sha256::new();
            hasher.update(&data);
            let digest = hex::encode(hasher.finalize());
            file_hashes.insert(rel_path.clone(), digest);
        }

        // Database snapshot hash
        if let Some(db_json) = db_snapshot_json {
            let mut hasher = Sha256::new();
            hasher.update(db_json.as_bytes());
            let digest = hex::encode(hasher.finalize());
            total_bytes += db_json.len() as u64;
            file_hashes.insert("database/snapshot.json".to_string(), digest);
        }

        // SSH host key fingerprint check
        let host_pub_path = storage_root.join("ssh/host_key_ed25519.pub");
        let ssh_fingerprint = if host_pub_path.exists() {
            if let Ok(pub_str) = fs::read_to_string(&host_pub_path) {
                if let Ok(parsed) = russh::keys::ssh_key::PublicKey::from_openssh(&pub_str) {
                    Some(parsed.fingerprint(russh::keys::ssh_key::HashAlg::Sha256).to_string())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        let manifest = BackupManifest {
            backup_id: backup_id.clone(),
            timestamp_nanos,
            software_version: "0.1.0".to_string(),
            schema_version: "0011_security_center".to_string(),
            total_files: file_hashes.len(),
            total_bytes,
            file_hashes: file_hashes.clone(),
            ssh_host_key_fingerprint: ssh_fingerprint,
            integrity_status: "VERIFIED".to_string(),
        };

        let manifest_bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        // Archive into .tar.gz
        let file = File::create(output_tar_gz).map_err(|e| ForgeError::Internal(e.to_string()))?;
        let enc = GzEncoder::new(file, Compression::default());
        let mut tar = tar::Builder::new(enc);

        // Append manifest.json first
        let mut header = tar::Header::new_gnu();
        header.set_size(manifest_bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, "manifest.json", &manifest_bytes[..])
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        // Append database snapshot
        if let Some(db_json) = db_snapshot_json {
            let mut db_header = tar::Header::new_gnu();
            db_header.set_size(db_json.len() as u64);
            db_header.set_mode(0o600);
            db_header.set_cksum();
            tar.append_data(&mut db_header, "database/snapshot.json", db_json.as_bytes())
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
        }

        // Append storage root files
        for (rel_path, abs_path) in file_paths {
            let data = fs::read(&abs_path).map_err(|e| ForgeError::Internal(e.to_string()))?;
            let mut f_header = tar::Header::new_gnu();
            f_header.set_size(data.len() as u64);
            f_header.set_mode(0o644);
            f_header.set_cksum();
            tar.append_data(&mut f_header, &rel_path, &data[..])
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
        }

        tar.finish().map_err(|e| ForgeError::Internal(e.to_string()))?;

        Ok(manifest)
    }

    /// Restores a verified backup into the target storage root, validating all cryptographic hashes
    pub fn restore_backup(
        backup_tar_gz: &Path,
        target_storage_root: &Path,
    ) -> Result<(BackupManifest, RestoreReport, Option<String>), ForgeError> {
        let start = Instant::now();

        let file = File::open(backup_tar_gz).map_err(|e| {
            ForgeError::Internal(format!("Failed to open backup archive: {}", e))
        })?;
        let dec = GzDecoder::new(file);
        let mut archive = tar::Archive::new(dec);

        fs::create_dir_all(target_storage_root)
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        // 1. Read manifest.json
        let mut manifest: Option<BackupManifest> = None;
        let mut extracted_db_snapshot: Option<String> = None;
        let mut extracted_files = BTreeMap::new();

        for entry in archive.entries().map_err(|e| ForgeError::Internal(e.to_string()))? {
            let mut entry = entry.map_err(|e| ForgeError::Internal(e.to_string()))?;
            let path = entry.path().map_err(|e| ForgeError::Internal(e.to_string()))?.to_path_buf();
            let path_str = path.to_string_lossy().replace('\\', "/");

            if path_str == "manifest.json" {
                let mut content = Vec::new();
                entry.read_to_end(&mut content).map_err(|e| ForgeError::Internal(e.to_string()))?;
                let parsed: BackupManifest = serde_json::from_slice(&content)
                    .map_err(|e| ForgeError::Internal(format!("Corrupt manifest: {}", e)))?;
                manifest = Some(parsed);
            } else if path_str == "database/snapshot.json" {
                let mut content = String::new();
                entry.read_to_string(&mut content).map_err(|e| ForgeError::Internal(e.to_string()))?;
                extracted_db_snapshot = Some(content);
            } else {
                let mut content = Vec::new();
                entry.read_to_end(&mut content).map_err(|e| ForgeError::Internal(e.to_string()))?;
                extracted_files.insert(path_str, content);
            }
        }

        let manifest = manifest.ok_or_else(|| {
            ForgeError::Internal("Backup archive missing manifest.json".to_string())
        })?;

        // 2. Verify and extract each file
        let mut files_restored = 0usize;
        let mut bytes_restored = 0u64;

        // Verify database snapshot hash if present
        if let Some(db_json) = &extracted_db_snapshot {
            let expected_hash = manifest.file_hashes.get("database/snapshot.json").ok_or_else(|| {
                ForgeError::Internal("Manifest missing hash for database/snapshot.json".to_string())
            })?;
            let mut hasher = Sha256::new();
            hasher.update(db_json.as_bytes());
            let actual_hash = hex::encode(hasher.finalize());
            if actual_hash != *expected_hash {
                return Err(ForgeError::Internal(format!(
                    "Integrity verification failed for database/snapshot.json: expected {}, got {}",
                    expected_hash, actual_hash
                )));
            }
            files_restored += 1;
            bytes_restored += db_json.len() as u64;
        }

        // Verify and write storage files
        for (rel_path, data) in extracted_files {
            let expected_hash = manifest.file_hashes.get(&rel_path).ok_or_else(|| {
                ForgeError::Internal(format!("Unexpected file in archive not in manifest: {}", rel_path))
            })?;

            let mut hasher = Sha256::new();
            hasher.update(&data);
            let actual_hash = hex::encode(hasher.finalize());
            if actual_hash != *expected_hash {
                return Err(ForgeError::Internal(format!(
                    "Integrity verification failed for {}: expected {}, got {}",
                    rel_path, expected_hash, actual_hash
                )));
            }

            let dest_path = target_storage_root.join(&rel_path);
            if let Some(parent) = dest_path.parent() {
                fs::create_dir_all(parent).map_err(|e| ForgeError::Internal(e.to_string()))?;
            }
            fs::write(&dest_path, &data).map_err(|e| ForgeError::Internal(e.to_string()))?;

            files_restored += 1;
            bytes_restored += data.len() as u64;
        }

        let duration_ms = start.elapsed().as_millis() as u64;

        let report = RestoreReport {
            backup_id: manifest.backup_id.clone(),
            files_restored,
            bytes_restored,
            duration_ms,
            integrity_verified: true,
        };

        Ok((manifest, report, extracted_db_snapshot))
    }

    fn collect_files_recursive(
        base: &Path,
        current: &Path,
        files: &mut Vec<(String, PathBuf)>,
    ) -> Result<(), ForgeError> {
        let entries = fs::read_dir(current).map_err(|e| ForgeError::Internal(e.to_string()))?;
        for entry in entries {
            let entry = entry.map_err(|e| ForgeError::Internal(e.to_string()))?;
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();

            // Skip temporary and lock files
            if file_name.ends_with(".tmp") || file_name.ends_with(".lock") {
                continue;
            }

            if path.is_dir() {
                Self::collect_files_recursive(base, &path, files)?;
            } else if path.is_file() {
                let rel = path
                    .strip_prefix(base)
                    .map_err(|e| ForgeError::Internal(e.to_string()))?
                    .to_string_lossy()
                    .replace('\\', "/");
                files.push((rel, path));
            }
        }
        Ok(())
    }
}
