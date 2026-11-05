use crate::core::error::ForgeError;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use ring::aead::{
    Aad, BoundKey, Nonce, NonceSequence, OpeningKey, SealingKey, UnboundKey, AES_256_GCM,
    NONCE_LEN,
};
use ring::pbkdf2;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Cursor, Read};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::time::Instant;
use uuid::Uuid;

const BACKUP_MAGIC: &[u8; 8] = b"SPNEBAK\x01";
const DEFAULT_PBKDF2_ITERATIONS: u32 = 100_000;
const SALT_LEN: usize = 16;
const HEADER_LEN: usize = 8 + 2 + 4 + SALT_LEN + NONCE_LEN; // 8 magic, 2 key_ver, 4 iterations, 16 salt, 12 nonce = 42

struct OneNonce(Option<[u8; NONCE_LEN]>);

impl NonceSequence for OneNonce {
    fn advance(&mut self) -> Result<Nonce, ring::error::Unspecified> {
        self.0
            .take()
            .map(Nonce::assume_unique_for_key)
            .ok_or(ring::error::Unspecified)
    }
}

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
        if let Some(parent) = output_tar_gz.parent() {
            fs::create_dir_all(parent).map_err(|e| ForgeError::Internal(e.to_string()))?;
        }

        let file = File::create(output_tar_gz).map_err(|e| ForgeError::Internal(e.to_string()))?;
        let enc = GzEncoder::new(file, Compression::default());
        let (manifest, _) = Self::create_archive_internal(storage_root, db_snapshot_json, enc)?;
        Ok(manifest)
    }

    /// Creates an authenticated, AEAD-encrypted backup archive using AES-256-GCM and PBKDF2 key derivation.
    pub fn create_encrypted_backup(
        storage_root: &Path,
        db_snapshot_json: Option<&str>,
        output_enc_path: &Path,
        passphrase: &str,
        key_version: u16,
    ) -> Result<BackupManifest, ForgeError> {
        if passphrase.trim().is_empty() {
            return Err(ForgeError::Internal(
                "Backup encryption passphrase cannot be empty".to_string(),
            ));
        }

        if let Some(parent) = output_enc_path.parent() {
            fs::create_dir_all(parent).map_err(|e| ForgeError::Internal(e.to_string()))?;
        }

        // 1. Pack tar.gz into memory buffer
        let mut tar_gz_buf = Vec::new();
        let enc = GzEncoder::new(&mut tar_gz_buf, Compression::default());
        let (manifest, _) = Self::create_archive_internal(storage_root, db_snapshot_json, enc)?;

        // 2. Generate random salt and nonce
        let rng = SystemRandom::new();
        let mut salt = [0u8; SALT_LEN];
        rng.fill(&mut salt)
            .map_err(|_| ForgeError::Internal("Failed to generate random salt".to_string()))?;

        let mut nonce = [0u8; NONCE_LEN];
        rng.fill(&mut nonce)
            .map_err(|_| ForgeError::Internal("Failed to generate random nonce".to_string()))?;

        // 3. Derive 256-bit encryption key
        let mut key_bytes = [0u8; 32];
        pbkdf2::derive(
            pbkdf2::PBKDF2_HMAC_SHA256,
            NonZeroU32::new(DEFAULT_PBKDF2_ITERATIONS).unwrap(),
            &salt,
            passphrase.as_bytes(),
            &mut key_bytes,
        );

        // 4. Assemble 42-byte header (authenticated as AAD)
        let mut header = Vec::with_capacity(HEADER_LEN);
        header.extend_from_slice(BACKUP_MAGIC);
        header.extend_from_slice(&key_version.to_be_bytes());
        header.extend_from_slice(&DEFAULT_PBKDF2_ITERATIONS.to_be_bytes());
        header.extend_from_slice(&salt);
        header.extend_from_slice(&nonce);

        // 5. Encrypt with AES-256-GCM
        let unbound_key = UnboundKey::new(&AES_256_GCM, &key_bytes)
            .map_err(|e| ForgeError::Internal(format!("AEAD key error: {:?}", e)))?;
        let mut sealing_key = SealingKey::new(unbound_key, OneNonce(Some(nonce)));

        sealing_key
            .seal_in_place_append_tag(Aad::from(&header), &mut tar_gz_buf)
            .map_err(|e| ForgeError::Internal(format!("AEAD encryption failed: {:?}", e)))?;

        // 6. Write [Header][Ciphertext + Tag]
        let mut final_output = Vec::with_capacity(header.len() + tar_gz_buf.len());
        final_output.extend_from_slice(&header);
        final_output.extend_from_slice(&tar_gz_buf);

        fs::write(output_enc_path, final_output)
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        Ok(manifest)
    }

    /// Restores a verified unencrypted backup archive (.tar.gz) into target storage root
    pub fn restore_backup(
        backup_tar_gz: &Path,
        target_storage_root: &Path,
    ) -> Result<(BackupManifest, RestoreReport, Option<String>), ForgeError> {
        let file = File::open(backup_tar_gz).map_err(|e| {
            ForgeError::Internal(format!("Failed to open backup archive: {}", e))
        })?;
        let dec = GzDecoder::new(file);
        Self::unpack_archive(dec, target_storage_root)
    }

    /// Restores an authenticated, AEAD-encrypted backup archive, verifying passphrases and tags
    pub fn restore_encrypted_backup(
        encrypted_path: &Path,
        target_storage_root: &Path,
        passphrase: &str,
    ) -> Result<(BackupManifest, RestoreReport, Option<String>), ForgeError> {
        let encrypted_bytes = fs::read(encrypted_path).map_err(|e| {
            ForgeError::Internal(format!("Failed to read encrypted backup: {}", e))
        })?;

        let decrypted_tar_gz = Self::decrypt_backup_payload(&encrypted_bytes, passphrase)?;
        let cursor = Cursor::new(decrypted_tar_gz);
        let dec = GzDecoder::new(cursor);
        Self::unpack_archive(dec, target_storage_root)
    }

    /// Rotates the encryption key/passphrase of an existing backup without writing decrypted files to disk
    pub fn rotate_backup_key(
        encrypted_path: &Path,
        output_path: &Path,
        old_passphrase: &str,
        new_passphrase: &str,
        new_key_version: u16,
    ) -> Result<(), ForgeError> {
        if new_passphrase.trim().is_empty() {
            return Err(ForgeError::Internal(
                "New backup encryption passphrase cannot be empty".to_string(),
            ));
        }

        let encrypted_bytes = fs::read(encrypted_path).map_err(|e| {
            ForgeError::Internal(format!("Failed to read source encrypted backup: {}", e))
        })?;

        // 1. Decrypt payload with old passphrase
        let mut plaintext_tar_gz = Self::decrypt_backup_payload(&encrypted_bytes, old_passphrase)?;

        // 2. Generate fresh salt and nonce
        let rng = SystemRandom::new();
        let mut new_salt = [0u8; SALT_LEN];
        rng.fill(&mut new_salt)
            .map_err(|_| ForgeError::Internal("Failed to generate random salt".to_string()))?;

        let mut new_nonce = [0u8; NONCE_LEN];
        rng.fill(&mut new_nonce)
            .map_err(|_| ForgeError::Internal("Failed to generate random nonce".to_string()))?;

        // 3. Derive new 256-bit encryption key
        let mut new_key_bytes = [0u8; 32];
        pbkdf2::derive(
            pbkdf2::PBKDF2_HMAC_SHA256,
            NonZeroU32::new(DEFAULT_PBKDF2_ITERATIONS).unwrap(),
            &new_salt,
            new_passphrase.as_bytes(),
            &mut new_key_bytes,
        );

        // 4. Assemble new 42-byte header
        let mut new_header = Vec::with_capacity(HEADER_LEN);
        new_header.extend_from_slice(BACKUP_MAGIC);
        new_header.extend_from_slice(&new_key_version.to_be_bytes());
        new_header.extend_from_slice(&DEFAULT_PBKDF2_ITERATIONS.to_be_bytes());
        new_header.extend_from_slice(&new_salt);
        new_header.extend_from_slice(&new_nonce);

        // 5. Seal with new key
        let unbound_key = UnboundKey::new(&AES_256_GCM, &new_key_bytes)
            .map_err(|e| ForgeError::Internal(format!("AEAD key error: {:?}", e)))?;
        let mut sealing_key = SealingKey::new(unbound_key, OneNonce(Some(new_nonce)));

        sealing_key
            .seal_in_place_append_tag(Aad::from(&new_header), &mut plaintext_tar_gz)
            .map_err(|e| ForgeError::Internal(format!("AEAD re-encryption failed: {:?}", e)))?;

        // 6. Write out rotated archive
        let mut rotated_output = Vec::with_capacity(new_header.len() + plaintext_tar_gz.len());
        rotated_output.extend_from_slice(&new_header);
        rotated_output.extend_from_slice(&plaintext_tar_gz);

        fs::write(output_path, rotated_output).map_err(|e| ForgeError::Internal(e.to_string()))?;

        Ok(())
    }

    /// Internal helper to decrypt ciphertext buffer using authenticated AEAD
    fn decrypt_backup_payload(
        encrypted_bytes: &[u8],
        passphrase: &str,
    ) -> Result<Vec<u8>, ForgeError> {
        if encrypted_bytes.len() < HEADER_LEN + 16 {
            return Err(ForgeError::Internal(
                "Invalid backup archive: file too short".to_string(),
            ));
        }

        // Validate magic bytes
        if &encrypted_bytes[0..8] != BACKUP_MAGIC {
            return Err(ForgeError::Internal(
                "Invalid backup archive: missing SPNEBAK header magic".to_string(),
            ));
        }

        let header = &encrypted_bytes[0..HEADER_LEN];
        let _key_version = u16::from_be_bytes([header[8], header[9]]);
        let iterations = u32::from_be_bytes([header[10], header[11], header[12], header[13]]);

        if iterations == 0 || iterations > 2_000_000 {
            return Err(ForgeError::Internal(
                "Invalid backup archive: unsupported KDF iterations".to_string(),
            ));
        }

        let salt = &header[14..30];
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&header[30..42]);

        // Derive key
        let mut key_bytes = [0u8; 32];
        pbkdf2::derive(
            pbkdf2::PBKDF2_HMAC_SHA256,
            NonZeroU32::new(iterations).unwrap(),
            salt,
            passphrase.as_bytes(),
            &mut key_bytes,
        );

        let unbound_key = UnboundKey::new(&AES_256_GCM, &key_bytes)
            .map_err(|e| ForgeError::Internal(format!("AEAD key error: {:?}", e)))?;
        let mut opening_key = OpeningKey::new(unbound_key, OneNonce(Some(nonce)));

        let mut in_out = encrypted_bytes[HEADER_LEN..].to_vec();
        let decrypted_slice = opening_key
            .open_in_place(Aad::from(header), &mut in_out)
            .map_err(|_| {
                ForgeError::Internal(
                    "AEAD decryption failed: invalid passphrase or corrupted backup ciphertext"
                        .to_string(),
                )
            })?;

        Ok(decrypted_slice.to_vec())
    }

    /// Sanitizes relative archive path to strictly prevent directory traversal attacks
    pub fn sanitize_restore_path(
        dest_root: &Path,
        rel_path: &str,
    ) -> Result<PathBuf, ForgeError> {
        if rel_path.trim().is_empty() {
            return Err(ForgeError::Internal(
                "Empty path in backup entry".to_string(),
            ));
        }

        let normalized = rel_path.replace('\\', "/");
        if normalized.starts_with('/') || normalized.starts_with('\\') || normalized.contains(':') {
            return Err(ForgeError::Internal(format!(
                "Absolute, drive-letter, or rooted path rejected: {}",
                rel_path
            )));
        }

        let path = Path::new(&normalized);
        for comp in path.components() {
            match comp {
                std::path::Component::ParentDir => {
                    return Err(ForgeError::Internal(format!(
                        "Path traversal attempt ('..') rejected: {}",
                        rel_path
                    )));
                }
                std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                    return Err(ForgeError::Internal(format!(
                        "Absolute path component rejected: {}",
                        rel_path
                    )));
                }
                std::path::Component::Normal(c) => {
                    let s = c.to_string_lossy().to_uppercase();
                    if matches!(
                        s.as_str(),
                        "CON"
                            | "PRN"
                            | "AUX"
                            | "NUL"
                            | "COM1"
                            | "COM2"
                            | "COM3"
                            | "COM4"
                            | "COM5"
                            | "COM6"
                            | "COM7"
                            | "COM8"
                            | "COM9"
                            | "LPT1"
                            | "LPT2"
                            | "LPT3"
                            | "LPT4"
                            | "LPT5"
                            | "LPT6"
                            | "LPT7"
                            | "LPT8"
                            | "LPT9"
                    ) {
                        return Err(ForgeError::Internal(format!(
                            "Reserved device name rejected: {}",
                            rel_path
                        )));
                    }
                }
                _ => {}
            }
        }

        Ok(dest_root.join(path))
    }

    /// Internal shared logic to package files into a tar builder
    fn create_archive_internal<W: std::io::Write>(
        storage_root: &Path,
        db_snapshot_json: Option<&str>,
        writer: W,
    ) -> Result<(BackupManifest, W), ForgeError> {
        let backup_id = Uuid::new_v4().to_string();
        let timestamp_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

        let mut file_hashes = BTreeMap::new();
        let mut total_bytes = 0u64;

        let mut file_paths = Vec::new();
        if storage_root.exists() {
            Self::collect_files_recursive(storage_root, storage_root, &mut file_paths)?;
        }

        for (rel_path, abs_path) in &file_paths {
            let data = fs::read(abs_path).map_err(|e| ForgeError::Internal(e.to_string()))?;
            total_bytes += data.len() as u64;
            let mut hasher = Sha256::new();
            hasher.update(&data);
            let digest = hex::encode(hasher.finalize());
            file_hashes.insert(rel_path.clone(), digest);
        }

        if let Some(db_json) = db_snapshot_json {
            let mut hasher = Sha256::new();
            hasher.update(db_json.as_bytes());
            let digest = hex::encode(hasher.finalize());
            total_bytes += db_json.len() as u64;
            file_hashes.insert("database/snapshot.json".to_string(), digest);
        }

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
            backup_id,
            timestamp_nanos,
            software_version: "0.1.0".to_string(),
            schema_version: "0011_security_center".to_string(),
            total_files: file_hashes.len(),
            total_bytes,
            file_hashes,
            ssh_host_key_fingerprint: ssh_fingerprint,
            integrity_status: "VERIFIED".to_string(),
        };

        let manifest_bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let mut tar = tar::Builder::new(writer);

        let mut header = tar::Header::new_gnu();
        header.set_size(manifest_bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, "manifest.json", &manifest_bytes[..])
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if let Some(db_json) = db_snapshot_json {
            let mut db_header = tar::Header::new_gnu();
            db_header.set_size(db_json.len() as u64);
            db_header.set_mode(0o600);
            db_header.set_cksum();
            tar.append_data(&mut db_header, "database/snapshot.json", db_json.as_bytes())
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
        }

        for (rel_path, abs_path) in file_paths {
            let data = fs::read(&abs_path).map_err(|e| ForgeError::Internal(e.to_string()))?;
            let mut f_header = tar::Header::new_gnu();
            f_header.set_size(data.len() as u64);
            f_header.set_mode(0o644);
            f_header.set_cksum();
            tar.append_data(&mut f_header, &rel_path, &data[..])
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
        }

        let writer = tar.into_inner().map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok((manifest, writer))
    }

    /// Internal shared logic to unpack tar archive, validating traversal limits and hashes
    fn unpack_archive<R: Read>(
        reader: R,
        target_storage_root: &Path,
    ) -> Result<(BackupManifest, RestoreReport, Option<String>), ForgeError> {
        let start = Instant::now();
        let mut archive = tar::Archive::new(reader);

        fs::create_dir_all(target_storage_root)
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

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

        let mut files_restored = 0usize;
        let mut bytes_restored = 0u64;

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

            // Path traversal defense
            let dest_path = Self::sanitize_restore_path(target_storage_root, &rel_path)?;
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
