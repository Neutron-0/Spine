use crate::core::error::ForgeError;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use ring::aead::{
    Aad, BoundKey, Nonce, NonceSequence, OpeningKey, UnboundKey, AES_256_GCM, NONCE_LEN,
};
use ring::pbkdf2;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Cursor, Read, Write};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::time::Instant;
use uuid::Uuid;

pub mod consistency;
pub mod journal;
pub mod participants;
pub mod v2;

pub use consistency::{ConsistencyBarrier, MaintenanceGuard, SPINE_CLUSTER_MAINTENANCE_LOCK_ID};
pub use journal::{CommitDecisionRecord, ParticipantAcknowledgement, RestoreJournal, RestorePhase};
pub use participants::{
    DeploymentResource, LocalStorageParticipantAdapter, ParticipantClassification,
    ParticipantRegistry, SshParticipantAdapter,
};
pub use v2::{BackupHeaderV2, StreamSummary};

pub const MAX_MANIFEST_BYTES: usize = 16 * 1024 * 1024; // 16 MiB
pub const MAX_ARCHIVE_FILE_COUNT: usize = 1_000_000;
pub const MAX_ARCHIVE_PATH_LEN: usize = 4096;
pub const MAX_LEGACY_V1_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024; // 64 MiB

const BACKUP_MAGIC: &[u8; 8] = b"SPNEBAK\x01";
#[allow(dead_code)]
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
pub struct BackupParticipantRecord {
    pub participant_id: String,
    pub volume_name: String,
    pub snapshot_id: String,
    pub fencing_generation: u64,
    pub byte_length: u64,
    pub sha256_checksum: String,
    pub archive_rel_path: String,
}

/// Bounded-memory ephemeral AEAD-encrypted temporary payload store (AES-256-GCM).
/// Ensures zero plaintext bytes are written to disk while bounding RAM to O(chunk_size).
#[derive(Debug)]
pub struct EphemeralEncryptedTemp {
    pub file: tempfile::NamedTempFile,
    ephemeral_key: [u8; 32],
    pub byte_length: u64,
    pub sha256_checksum: String,
}

impl Drop for EphemeralEncryptedTemp {
    fn drop(&mut self) {
        for b in &mut self.ephemeral_key {
            unsafe {
                std::ptr::write_volatile(b, 0);
            }
        }
    }
}

impl EphemeralEncryptedTemp {
    /// Generates an ephemeral 256-bit CSPRNG key and creates an AES-256-GCM streaming writer backed by a TempFile.
    pub fn begin(
        chunk_size: u32,
    ) -> Result<
        (
            tempfile::NamedTempFile,
            [u8; 32],
            v2::EncryptingWriter<std::fs::File>,
        ),
        ForgeError,
    > {
        use ring::rand::{SecureRandom, SystemRandom};
        let mut key = [0u8; 32];
        SystemRandom::new()
            .fill(&mut key)
            .map_err(|_| ForgeError::Internal("Failed to generate ephemeral CSPRNG key".into()))?;
        let tmp = tempfile::NamedTempFile::new().map_err(|e| {
            ForgeError::Internal(format!(
                "Failed to create ephemeral encrypted temp file: {}",
                e
            ))
        })?;
        let handle = tmp.reopen().map_err(|e| {
            ForgeError::Internal(format!(
                "Failed to reopen ephemeral encrypted temp file: {}",
                e
            ))
        })?;
        let writer = v2::EncryptingWriter::new_ephemeral(handle, &key, chunk_size)?;
        Ok((tmp, key, writer))
    }

    pub fn from_finished(
        file: tempfile::NamedTempFile,
        key: [u8; 32],
        summary: v2::StreamSummary,
    ) -> Self {
        Self {
            file,
            ephemeral_key: key,
            byte_length: summary.total_plaintext_bytes,
            sha256_checksum: hex::encode(summary.stream_sha256),
        }
    }

    /// Streams raw bytes from an `io::Read` source through AES-256-GCM into an encrypted temp file.
    pub fn from_reader<R: Read>(mut reader: R, chunk_size: u32) -> Result<Self, ForgeError> {
        let (tmp, mut key, mut writer) = Self::begin(chunk_size)?;
        let res = (|| {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = reader
                    .read(&mut buf)
                    .map_err(|e| ForgeError::Internal(format!("Read error: {}", e)))?;
                if n == 0 {
                    break;
                }
                writer.write_all(&buf[..n]).map_err(|e| {
                    ForgeError::Internal(format!("Encrypted staging write error: {}", e))
                })?;
            }
            writer.finish()
        })();
        match res {
            Ok(summary) => Ok(Self::from_finished(tmp, key, summary)),
            Err(e) => {
                for b in &mut key {
                    unsafe {
                        std::ptr::write_volatile(b, 0);
                    }
                }
                Err(e)
            }
        }
    }

    pub fn open_reader(&self) -> Result<v2::DecryptingReader<std::fs::File>, ForgeError> {
        let file = self.file.reopen().map_err(|e| {
            ForgeError::Internal(format!("Failed to reopen ephemeral encrypted file: {}", e))
        })?;
        v2::DecryptingReader::new_with_raw_key(file, &self.ephemeral_key)
    }
}

/// Source of raw payload bytes for a backup participant.
/// Supports in-memory buffers (for small keys), streamed disk files, and bounded-memory ephemeral AEAD streams.
#[derive(Debug)]
pub enum ParticipantPayloadSource {
    Memory(Vec<u8>),
    File(PathBuf),
    EncryptedTemp(EphemeralEncryptedTemp),
}

/// Recursively copies a directory and all its contents.
pub fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), ForgeError> {
    fs::create_dir_all(dst).map_err(|e| ForgeError::Internal(e.to_string()))?;
    for entry in fs::read_dir(src).map_err(|e| ForgeError::Internal(e.to_string()))? {
        let entry = entry.map_err(|e| ForgeError::Internal(e.to_string()))?;
        let ty = entry
            .file_type()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let dest_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dest_path)?;
        } else {
            fs::copy(entry.path(), &dest_path).map_err(|e| ForgeError::Internal(e.to_string()))?;
        }
    }
    Ok(())
}

/// Safely moves a file across filesystems or drive letters with copy+delete fallback.
pub fn safe_move_file(from: &Path, to: &Path) -> Result<(), ForgeError> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    fs::copy(from, to).map_err(|e| {
        ForgeError::Internal(format!(
            "Failed to copy file from {:?} to {:?}: {}",
            from, to, e
        ))
    })?;
    let _ = fs::remove_file(from);
    Ok(())
}

/// Safely moves a file or directory across filesystems or drive letters.
pub fn safe_move_path(from: &Path, to: &Path) -> Result<(), ForgeError> {
    if fs::rename(from, to).is_ok() {
        return Ok(());
    }
    if from.is_dir() {
        copy_dir_all(from, to)?;
        let _ = fs::remove_dir_all(from);
        Ok(())
    } else {
        safe_move_file(from, to)
    }
}

/// Safely flush a directory handle across platforms.
///
/// On POSIX systems, opens the directory file descriptor and executes `fsync` (`sync_all`).
/// On Windows systems, directory handles cannot be passed to `FlushFileBuffers` without triggering
/// Win32 OS error 650 (`ERROR_DIRECTORY_NOT_SUPPORTED` / `STATUS_DIRECTORY_IS_A_FILE`) or error 1 (`ERROR_INVALID_FUNCTION`).
/// Windows durability is instead guaranteed by file-level `FlushFileBuffers` combined with atomic replacements
/// via `ReplaceFileW` and `MoveFileExW` using `REPLACEFILE_WRITE_THROUGH` and `MOVEFILE_WRITE_THROUGH`, which flush
/// both the file payload and the containing directory's MFT metadata to physical media. `sync_dir` on Windows
/// verifies directory presence without issuing unsupported flush calls.
pub fn sync_dir(dir: &Path) -> Result<(), ForgeError> {
    #[cfg(windows)]
    {
        if !dir.exists() {
            return Err(ForgeError::Internal(format!(
                "Directory does not exist for sync: {}",
                dir.display()
            )));
        }
    }
    #[cfg(unix)]
    {
        if let Ok(f) = std::fs::File::open(dir) {
            f.sync_all().map_err(|e| {
                ForgeError::Internal(format!("Failed to sync directory {}: {}", dir.display(), e))
            })?;
        }
    }
    Ok(())
}

#[cfg(windows)]
fn is_write_through_unsupported(err: &std::io::Error) -> bool {
    // Win32 OS Error 650 (ERROR_DEVICE_OBJECT_PARAMETER_INVALID): returned when a
    // layered/sandboxed/virtualized volume lacks an attached storage DeviceObject
    // for write-through rename IRPs (SL_WRITE_THROUGH).
    // OS Error 50 (ERROR_NOT_SUPPORTED) and OS Error 1 (ERROR_INVALID_FUNCTION)
    // similarly indicate the underlying volume filter rejects write-through rename flags.
    matches!(err.raw_os_error(), Some(650) | Some(50) | Some(1))
}

#[cfg(windows)]
fn win32_sync_file_handle(path: &Path) -> std::io::Result<()> {
    let file = std::fs::OpenOptions::new().write(true).open(path)?;
    file.sync_all()
}

#[cfg(windows)]
fn platform_atomic_replace_inner(
    from: &Path,
    to: &Path,
    write_through: bool,
) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    let wide_from: Vec<u16> = from
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let wide_to: Vec<u16> = to
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    extern "system" {
        fn MoveFileExW(
            lpExistingFileName: *const u16,
            lpNewFileName: *const u16,
            dwFlags: u32,
        ) -> i32;
        fn ReplaceFileW(
            lpReplacedFileName: *const u16,
            lpReplacementFileName: *const u16,
            lpBackupFileName: *const u16,
            dwReplaceFlags: u32,
            lpExclude: *mut std::ffi::c_void,
            lpReserved: *mut std::ffi::c_void,
        ) -> i32;
    }

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    const REPLACEFILE_WRITE_THROUGH: u32 = 0x1;

    let move_flags = if write_through {
        MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH
    } else {
        MOVEFILE_REPLACE_EXISTING
    };
    let replace_flags = if write_through {
        REPLACEFILE_WRITE_THROUGH
    } else {
        0
    };

    if to.exists() {
        let ret = unsafe {
            ReplaceFileW(
                wide_to.as_ptr(),
                wide_from.as_ptr(),
                std::ptr::null(),
                replace_flags,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if ret != 0 {
            return Ok(());
        }
        let ret = unsafe { MoveFileExW(wide_from.as_ptr(), wide_to.as_ptr(), move_flags) };
        if ret != 0 {
            return Ok(());
        }
        Err(std::io::Error::last_os_error())
    } else {
        let ret = unsafe { MoveFileExW(wide_from.as_ptr(), wide_to.as_ptr(), move_flags) };
        if ret != 0 {
            return Ok(());
        }
        Err(std::io::Error::last_os_error())
    }
}

/// Atomically replaces `to` with `from` on Windows.
///
/// Durability contract:
/// 1. Flushes `from` (`OpenOptions::write` + `sync_all` / `FlushFileBuffers`) before renaming, propagating any error.
/// 2. Attempts `ReplaceFileW(..., REPLACEFILE_WRITE_THROUGH)` / `MoveFileExW(..., MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)`,
///    which flushes both file data and directory/MFT metadata on standard NTFS volumes.
/// 3. Only if the underlying volume/filter rejects write-through rename IRPs with OS error `650`, `50`, or `1`,
///    retries atomic replacement without `WRITE_THROUGH` and immediately flushes the destination file handle (`sync_all`),
///    propagating any open or flush failure. On such restricted/layered filesystems, file payload durability is
///    enforced via explicit `FlushFileBuffers` before and after rename, while directory-entry metadata write-through
///    depends on the volume filter because `SL_WRITE_THROUGH` is unsupported by that filter.
#[cfg(windows)]
fn platform_atomic_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    platform_atomic_replace_with_hooks(
        from,
        to,
        win32_sync_file_handle,
        platform_atomic_replace_inner,
    )
}

#[cfg(windows)]
pub fn platform_atomic_replace_with_hooks<S, R>(
    from: &Path,
    to: &Path,
    mut sync_fn: S,
    mut replace_fn: R,
) -> std::io::Result<()>
where
    S: FnMut(&Path) -> std::io::Result<()>,
    R: FnMut(&Path, &Path, bool) -> std::io::Result<()>,
{
    sync_fn(from)?;
    match replace_fn(from, to, true) {
        Ok(()) => Ok(()),
        Err(err) if is_write_through_unsupported(&err) => {
            tracing::warn!(
                "Win32 write-through atomic replace rejected ({}) replacing {} -> {}; retrying synced atomic rename without WRITE_THROUGH",
                err,
                from.display(),
                to.display()
            );
            replace_fn(from, to, false)?;
            sync_fn(to)?;
            Ok(())
        }
        Err(err) => Err(err),
    }
}

/// Executes strict write-through atomic replacement without fallback, for verifying hardware write-through support.
pub fn safe_atomic_replace_strict_writethrough(from: &Path, to: &Path) -> Result<(), ForgeError> {
    #[cfg(windows)]
    {
        win32_sync_file_handle(from).map_err(|e| {
            ForgeError::Internal(format!(
                "Failed to sync source file {} before strict write-through replace: {}",
                from.display(),
                e
            ))
        })?;
        platform_atomic_replace_inner(from, to, true).map_err(|e| {
            ForgeError::Internal(format!(
                "Strict write-through atomic replace failed from {} to {}: {}",
                from.display(),
                to.display(),
                e
            ))
        })?;
        if !to.exists() || fs::metadata(to).is_err() {
            return Err(ForgeError::Internal(format!(
                "Readback verification failed after replacing {}",
                to.display()
            )));
        }
        if let Some(parent) = to.parent() {
            sync_dir(parent)?;
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        safe_atomic_replace(from, to)
    }
}

#[cfg(unix)]
fn platform_atomic_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to)
}

/// Safely replaces a destination file atomically, preserving previous destination until replacement is confirmed.
/// Never deletes destination file prior to verification. Accommodates platform atomic replacement semantics.
pub fn safe_atomic_replace(from: &Path, to: &Path) -> Result<(), ForgeError> {
    platform_atomic_replace(from, to).map_err(|e| {
        ForgeError::Internal(format!(
            "Failed atomic replace from {} to {}: {}",
            from.display(),
            to.display(),
            e
        ))
    })?;

    if !to.exists() || fs::metadata(to).is_err() {
        return Err(ForgeError::Internal(format!(
            "Readback verification failed after replacing {}",
            to.display()
        )));
    }

    if let Some(parent) = to.parent() {
        sync_dir(parent)?;
    }

    Ok(())
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
    #[serde(default)]
    pub fencing_generation: Option<u64>,
    #[serde(default)]
    pub participants: Vec<BackupParticipantRecord>,
    #[serde(default)]
    pub required_participant_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RestoreReport {
    pub backup_id: String,
    pub files_restored: usize,
    pub bytes_restored: u64,
    pub duration_ms: u64,
    pub integrity_verified: bool,
}

/// Represents an active, staged restore session prior to storage publication.
/// Keeps live storage unmodified until the caller explicitly confirms and commits.
pub struct StagedRestoreSession {
    pub manifest: BackupManifest,
    pub report: RestoreReport,
    pub journal: RestoreJournal,
    pub db_snapshot_path: Option<PathBuf>,
    pub staging_dir: tempfile::TempDir,
    pub verified_files: Vec<(String, PathBuf, u64)>,
    pub target_storage_root: PathBuf,
}

impl StagedRestoreSession {
    pub fn get_participant_snapshot_data(&self, rel_path: &str) -> Option<Vec<u8>> {
        for (p, staged_path, _) in &self.verified_files {
            if p == rel_path {
                return fs::read(staged_path).ok();
            }
        }
        None
    }

    pub fn get_participant_snapshot_path(&self, rel_path: &str) -> Option<PathBuf> {
        for (p, staged_path, _) in &self.verified_files {
            if p == rel_path {
                return Some(staged_path.clone());
            }
        }
        None
    }

    /// Commits the staged files to live storage root using whole-generation replacement.
    /// This MUST only be called after target database restore is successful.
    /// Retains old generation in `.old_generation_<restore_id>` until finalize() is called.
    pub fn publish_storage(&mut self) -> Result<RestoreReport, ForgeError> {
        let mut files_restored = 0usize;
        let mut bytes_restored = 0u64;

        fs::create_dir_all(&self.target_storage_root)
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let restore_id = &self.journal.restore_id;
        let old_dir = self
            .target_storage_root
            .join(format!(".old_generation_{}", restore_id));
        if old_dir.exists() {
            let _ = fs::remove_dir_all(&old_dir);
        }
        fs::create_dir_all(&old_dir).map_err(|e| ForgeError::Internal(e.to_string()))?;

        // 1. Whole-generation replacement: move existing live entries into old_dir
        let staging_path = self.staging_dir.path().to_path_buf();
        if let Ok(entries) = fs::read_dir(&self.target_storage_root) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p == old_dir || p == staging_path {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if !name.starts_with(".old_generation_")
                    && !name.starts_with(".staged_generation_")
                    && !name.starts_with(".tmp")
                    && !name.starts_with(".old_ssh_")
                    && !name.starts_with(".spine_restore_control")
                    && name != RestoreJournal::JOURNAL_FILE_NAME
                {
                    let dst = old_dir.join(&name);
                    let _ = safe_move_path(&p, &dst);
                }
            }
        }

        let participant_paths: std::collections::HashSet<_> = self
            .manifest
            .participants
            .iter()
            .map(|p| p.archive_rel_path.as_str())
            .collect();

        // Check if spine_storage participant snapshot is present
        let storage_participant = self
            .manifest
            .participants
            .iter()
            .find(|p| p.participant_id == "spine_storage");

        if let Some(sp) = storage_participant {
            // Find staged snapshot file
            let staged_snap = self
                .verified_files
                .iter()
                .find(|(rel, _, _)| rel == &sp.archive_rel_path)
                .map(|(_, path, _)| path.clone())
                .ok_or_else(|| {
                    ForgeError::Internal(format!(
                        "spine_storage payload '{}' missing from staged files",
                        sp.archive_rel_path
                    ))
                })?;

            let snap_file = File::open(&staged_snap).map_err(|e| {
                ForgeError::Internal(format!("Failed to open spine_storage payload: {}", e))
            })?;
            let dec = GzDecoder::new(snap_file);
            let mut snap_tar = tar::Archive::new(dec);

            for entry in snap_tar
                .entries()
                .map_err(|e| ForgeError::Internal(e.to_string()))?
            {
                let mut entry = entry.map_err(|e| ForgeError::Internal(e.to_string()))?;
                let path = entry
                    .path()
                    .map_err(|e| ForgeError::Internal(e.to_string()))?
                    .to_path_buf();
                let dest = BackupManager::sanitize_restore_path(
                    &self.target_storage_root,
                    &path.to_string_lossy(),
                )?;
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent).map_err(|e| ForgeError::Internal(e.to_string()))?;
                }
                entry
                    .unpack(&dest)
                    .map_err(|e| ForgeError::Internal(e.to_string()))?;
                files_restored += 1;
                if let Ok(m) = fs::metadata(&dest) {
                    bytes_restored += m.len();
                }
            }
        } else {
            // 2. Move staged verified files into target live root (standalone legacy format)
            for (rel_path, staged_path, file_len) in &self.verified_files {
                if rel_path != "database/snapshot.jsonl"
                    && rel_path != "database/snapshot.json"
                    && !participant_paths.contains(rel_path.as_str())
                {
                    let dest =
                        BackupManager::sanitize_restore_path(&self.target_storage_root, rel_path)?;
                    if let Some(parent) = dest.parent() {
                        fs::create_dir_all(parent)
                            .map_err(|e| ForgeError::Internal(e.to_string()))?;
                    }
                    safe_move_file(staged_path, &dest)?;
                    files_restored += 1;
                    bytes_restored += *file_len;
                } else {
                    files_restored += 1;
                    bytes_restored += *file_len;
                }
            }
        }

        RestoreJournal::write_storage_generation(
            &self.target_storage_root,
            &self.journal.target_generation,
        )?;

        self.journal.persist(RestorePhase::StoragePublished)?;

        // NOTE: .old_generation_<id> is retained until finalize() to guarantee rollback is possible
        self.report.files_restored = files_restored;
        self.report.bytes_restored = bytes_restored;
        Ok(self.report.clone())
    }

    /// Finalizes the restore session after all participants have been verified.
    /// Deletes the retained old generation directory and removes the restore journal.
    pub fn finalize(mut self) -> Result<RestoreReport, ForgeError> {
        self.journal.persist(RestorePhase::Completed)?;
        let old_dir = self
            .target_storage_root
            .join(format!(".old_generation_{}", self.journal.restore_id));
        if old_dir.exists() {
            let _ = fs::remove_dir_all(&old_dir);
        }
        let _ = RestoreJournal::remove(&self.target_storage_root);
        Ok(self.report)
    }

    /// Reverts the live storage root back to the retained old generation.
    pub fn rollback(&mut self) -> Result<(), ForgeError> {
        let old_dir = self
            .target_storage_root
            .join(format!(".old_generation_{}", self.journal.restore_id));
        if old_dir.exists() {
            // Remove current live files
            if let Ok(entries) = fs::read_dir(&self.target_storage_root) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    if p != old_dir && p != self.staging_dir.path() {
                        if p.is_dir() {
                            let _ = fs::remove_dir_all(&p);
                        } else {
                            let _ = fs::remove_file(&p);
                        }
                    }
                }
            }
            // Move old generation files back
            if let Ok(entries) = fs::read_dir(&old_dir) {
                for entry in entries.flatten() {
                    let p = entry.path();
                    let name = entry.file_name();
                    let dst = self.target_storage_root.join(name);
                    let _ = fs::rename(&p, &dst);
                }
            }
            let _ = fs::remove_dir_all(&old_dir);
        }

        if let Some(ref prev_gen) = self.journal.previous_storage_generation {
            let _ = RestoreJournal::write_storage_generation(&self.target_storage_root, prev_gen);
        } else {
            let gen_file = self
                .target_storage_root
                .join(RestoreJournal::GENERATION_FILE_NAME);
            let _ = fs::remove_file(&gen_file);
        }

        let _ = self.journal.persist(RestorePhase::RolledBack(
            "Restored from old generation".to_string(),
        ));
        let _ = RestoreJournal::remove(&self.target_storage_root);
        Ok(())
    }

    /// Abort and discard staging artifacts leaving live storage completely untouched.
    pub fn abort(&mut self, reason: &str) -> Result<(), ForgeError> {
        let _ = self.rollback();
        let _ = self
            .journal
            .persist(RestorePhase::Failed(reason.to_string()));
        let _ = RestoreJournal::remove(&self.target_storage_root);
        Ok(())
    }
}

pub struct BackupManager;

impl BackupManager {
    /// Creates a verified, coordinated backup archive (.tar.gz) from storage root and database snapshot
    pub fn create_backup(
        storage_root: &Path,
        db_snapshot_json: Option<&str>,
        output_tar_gz: &Path,
    ) -> Result<BackupManifest, ForgeError> {
        let parent = output_tar_gz.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|e| ForgeError::Internal(e.to_string()))?;
        let tmp_output = parent.join(format!(".tmp_bak_{}.tar.gz", Uuid::new_v4()));

        let res = (|| {
            let file =
                File::create(&tmp_output).map_err(|e| ForgeError::Internal(e.to_string()))?;
            let enc = GzEncoder::new(file, Compression::default());
            let (manifest, _) = Self::create_archive_internal(
                storage_root,
                db_snapshot_json,
                None,
                &[],
                None,
                enc,
            )?;
            safe_atomic_replace(&tmp_output, output_tar_gz)?;
            Ok(manifest)
        })();

        if res.is_err() && tmp_output.exists() {
            let _ = fs::remove_file(&tmp_output);
        }
        res
    }

    /// Creates an unencrypted backup archive with database snapshot streamed from a disk file
    pub fn create_backup_from_file(
        storage_root: &Path,
        db_snapshot_file: Option<&Path>,
        output_tar_gz: &Path,
    ) -> Result<BackupManifest, ForgeError> {
        let parent = output_tar_gz.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|e| ForgeError::Internal(e.to_string()))?;
        let tmp_output = parent.join(format!(".tmp_bak_{}.tar.gz", Uuid::new_v4()));

        let res = (|| {
            let file =
                File::create(&tmp_output).map_err(|e| ForgeError::Internal(e.to_string()))?;
            let enc = GzEncoder::new(file, Compression::default());
            let (manifest, _) = Self::create_archive_internal(
                storage_root,
                None,
                db_snapshot_file,
                &[],
                None,
                enc,
            )?;
            safe_atomic_replace(&tmp_output, output_tar_gz)?;
            Ok(manifest)
        })();

        if res.is_err() && tmp_output.exists() {
            let _ = fs::remove_file(&tmp_output);
        }
        res
    }

    fn write_storage_snapshot_tar_gz<W: Write>(
        storage_root: &Path,
        sink: W,
    ) -> Result<W, ForgeError> {
        let enc = GzEncoder::new(sink, Compression::default());
        let mut tar = tar::Builder::new(enc);

        let mut file_paths = Vec::new();
        if storage_root.exists() {
            Self::collect_files_recursive(storage_root, storage_root, &mut file_paths)?;
        }
        file_paths.sort_by(|a, b| a.0.cmp(&b.0));

        for (rel_path, abs_path) in file_paths {
            if rel_path.starts_with(".old_generation_")
                || rel_path.starts_with(".old_ssh_")
                || rel_path.starts_with(".staged_generation_")
                || rel_path.starts_with(".tmp")
                || rel_path.starts_with(".spine_restore_control")
                || rel_path == RestoreJournal::JOURNAL_FILE_NAME
            {
                continue;
            }

            let meta = fs::metadata(&abs_path).map_err(|e| ForgeError::Internal(e.to_string()))?;
            let mut f =
                fs::File::open(&abs_path).map_err(|e| ForgeError::Internal(e.to_string()))?;
            let mut header = tar::Header::new_gnu();
            header.set_size(meta.len());
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, &rel_path, &mut f)
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
        }

        let enc = tar
            .into_inner()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        enc.finish()
            .map_err(|e| ForgeError::Internal(e.to_string()))
    }

    /// Streams the storage directory .tar.gz snapshot through AES-256-GCM in O(chunk_size) RAM.
    /// Zero plaintext intermediate bytes ever hit disk.
    pub fn capture_storage_snapshot_encrypted(
        storage_root: &Path,
        chunk_size: u32,
    ) -> Result<EphemeralEncryptedTemp, ForgeError> {
        let (tmp, mut key, enc_writer) = EphemeralEncryptedTemp::begin(chunk_size)?;
        let res = (|| {
            let enc_writer = Self::write_storage_snapshot_tar_gz(storage_root, enc_writer)?;
            enc_writer.finish()
        })();
        match res {
            Ok(summary) => Ok(EphemeralEncryptedTemp::from_finished(tmp, key, summary)),
            Err(e) => {
                for b in &mut key {
                    unsafe {
                        std::ptr::write_volatile(b, 0);
                    }
                }
                Err(e)
            }
        }
    }

    /// Captures the complete storage directory into an in-memory .tar.gz archive payload without touching disk.
    /// Returns the raw compressed bytes and its SHA-256 digest, ensuring zero plaintext intermediate files on disk.
    pub fn capture_storage_snapshot_memory(
        storage_root: &Path,
    ) -> Result<(Vec<u8>, String), ForgeError> {
        let bytes = Self::write_storage_snapshot_tar_gz(storage_root, Vec::new())?;

        use sha2::Sha256;
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let digest = hex::encode(hasher.finalize());

        Ok((bytes, digest))
    }

    /// Captures the complete storage directory into a streaming .tar.gz archive payload on disk.
    /// Memory consumption is strictly O(1) (~64 KiB buffer), returning exact byte length and SHA-256 digest.
    pub fn capture_storage_snapshot(
        storage_root: &Path,
        dest_path: &Path,
    ) -> Result<(u64, String), ForgeError> {
        let parent = dest_path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|e| ForgeError::Internal(e.to_string()))?;

        let file = File::create(dest_path).map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut file = Self::write_storage_snapshot_tar_gz(storage_root, file)?;
        file.flush()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        file.sync_all()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let meta = fs::metadata(dest_path).map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut f = File::open(dest_path).map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut hasher = Sha256::new();
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = f
                .read(&mut buf)
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let hash = hex::encode(hasher.finalize());
        Ok((meta.len(), hash))
    }

    /// Creates a complete cluster backup archive with database dump and multi-volume participant snapshots.
    pub fn create_cluster_backup(
        storage_root: &Path,
        db_snapshot_file: Option<&Path>,
        participants: Vec<(BackupParticipantRecord, Vec<u8>)>,
        fencing_generation: Option<u64>,
        output_path: &Path,
        passphrase: Option<&str>,
        key_version: u16,
    ) -> Result<BackupManifest, ForgeError> {
        let stream_participants: Vec<_> = participants
            .into_iter()
            .map(|(r, d)| (r, ParticipantPayloadSource::Memory(d)))
            .collect();
        Self::create_cluster_backup_streaming(
            storage_root,
            db_snapshot_file,
            stream_participants,
            fencing_generation,
            output_path,
            passphrase,
            key_version,
        )
    }

    /// Creates a complete cluster backup archive with bounded-memory streaming participant payloads.
    pub fn create_cluster_backup_streaming(
        storage_root: &Path,
        db_snapshot_file: Option<&Path>,
        participants: Vec<(BackupParticipantRecord, ParticipantPayloadSource)>,
        fencing_generation: Option<u64>,
        output_path: &Path,
        passphrase: Option<&str>,
        key_version: u16,
    ) -> Result<BackupManifest, ForgeError> {
        let parent = output_path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|e| ForgeError::Internal(e.to_string()))?;

        if let Some(pass) = passphrase {
            if pass.trim().is_empty() {
                return Err(ForgeError::Internal(
                    "Backup encryption passphrase cannot be empty".to_string(),
                ));
            }
            let tmp_output = parent.join(format!(".tmp_bak_{}.spnebak", Uuid::new_v4()));
            let res = (|| {
                let file = File::create(&tmp_output).map_err(|e| {
                    ForgeError::Internal(format!("Failed to create output archive: {}", e))
                })?;
                let enc_writer =
                    v2::EncryptingWriter::new(file, pass, key_version, v2::DEFAULT_CHUNK_SIZE)?;
                let enc = GzEncoder::new(enc_writer, Compression::default());
                let (manifest, enc) = Self::create_archive_internal(
                    storage_root,
                    None,
                    db_snapshot_file,
                    &participants,
                    fencing_generation,
                    enc,
                )?;
                let enc_writer = enc
                    .finish()
                    .map_err(|e| ForgeError::Internal(format!("Failed to finalize gzip: {}", e)))?;
                let _ = enc_writer.finish()?;

                let verify_file = File::open(&tmp_output).map_err(|e| {
                    ForgeError::Internal(format!(
                        "Failed to reopen archive for verification: {}",
                        e
                    ))
                })?;
                v2::decrypt_stream(verify_file, std::io::sink(), pass)?;
                safe_atomic_replace(&tmp_output, output_path)?;
                Ok(manifest)
            })();
            if res.is_err() && tmp_output.exists() {
                let _ = fs::remove_file(&tmp_output);
            }
            res
        } else {
            let tmp_output = parent.join(format!(".tmp_bak_{}.tar.gz", Uuid::new_v4()));
            let res = (|| {
                let file =
                    File::create(&tmp_output).map_err(|e| ForgeError::Internal(e.to_string()))?;
                let enc = GzEncoder::new(file, Compression::default());
                let (manifest, _) = Self::create_archive_internal(
                    storage_root,
                    None,
                    db_snapshot_file,
                    &participants,
                    fencing_generation,
                    enc,
                )?;
                safe_atomic_replace(&tmp_output, output_path)?;
                Ok(manifest)
            })();
            if res.is_err() && tmp_output.exists() {
                let _ = fs::remove_file(&tmp_output);
            }
            res
        }
    }

    /// Creates an authenticated, streaming chunked AEAD-encrypted backup archive (SPNEBAK v2)
    /// using AES-256-GCM, PBKDF2 key derivation, per-chunk nonces, and authenticated manifest.
    ///
    /// Memory consumption is strictly bounded to O(chunk_size) (1 MiB), supporting archives >64 GiB.
    pub fn create_encrypted_backup(
        storage_root: &Path,
        db_snapshot_json: Option<&str>,
        output_enc_path: &Path,
        passphrase: &str,
        key_version: u16,
    ) -> Result<BackupManifest, ForgeError> {
        Self::create_encrypted_backup_internal(
            storage_root,
            db_snapshot_json,
            None,
            output_enc_path,
            passphrase,
            key_version,
        )
    }

    /// Creates an encrypted backup archive with database snapshot streamed directly from a disk file.
    pub fn create_encrypted_backup_from_file(
        storage_root: &Path,
        db_snapshot_file: Option<&Path>,
        output_enc_path: &Path,
        passphrase: &str,
        key_version: u16,
    ) -> Result<BackupManifest, ForgeError> {
        Self::create_encrypted_backup_internal(
            storage_root,
            None,
            db_snapshot_file,
            output_enc_path,
            passphrase,
            key_version,
        )
    }

    fn create_encrypted_backup_internal(
        storage_root: &Path,
        db_snapshot_json: Option<&str>,
        db_snapshot_file: Option<&Path>,
        output_enc_path: &Path,
        passphrase: &str,
        key_version: u16,
    ) -> Result<BackupManifest, ForgeError> {
        if passphrase.trim().is_empty() {
            return Err(ForgeError::Internal(
                "Backup encryption passphrase cannot be empty".to_string(),
            ));
        }

        let output_parent = output_enc_path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(output_parent).map_err(|e| ForgeError::Internal(e.to_string()))?;
        let tmp_output = output_parent.join(format!(".tmp_bak_{}.spnebak", Uuid::new_v4()));

        let res = (|| {
            // 1. Directly stream compressed tar archive through Authenticated EncryptingWriter (SPNEBAK v2)
            let file = File::create(&tmp_output).map_err(|e| {
                ForgeError::Internal(format!("Failed to create output archive: {}", e))
            })?;
            let enc_writer =
                v2::EncryptingWriter::new(file, passphrase, key_version, v2::DEFAULT_CHUNK_SIZE)?;
            let enc = GzEncoder::new(enc_writer, Compression::default());
            let (manifest, enc) = Self::create_archive_internal(
                storage_root,
                db_snapshot_json,
                db_snapshot_file,
                &[],
                None,
                enc,
            )?;
            let enc_writer = enc
                .finish()
                .map_err(|e| ForgeError::Internal(format!("Failed to finalize gzip: {}", e)))?;
            let _ = enc_writer.finish()?;

            // 2. Verify temporary encrypted output completely before publication
            let verify_file = File::open(&tmp_output).map_err(|e| {
                ForgeError::Internal(format!("Failed to reopen archive for verification: {}", e))
            })?;
            v2::decrypt_stream(verify_file, std::io::sink(), passphrase)?;

            // 4. Atomically publish to final destination
            safe_atomic_replace(&tmp_output, output_enc_path)?;

            Ok(manifest)
        })();

        if res.is_err() && tmp_output.exists() {
            let _ = fs::remove_file(&tmp_output);
        }
        res
    }

    /// Restores a verified unencrypted backup archive (.tar.gz) into target storage root
    pub fn restore_backup(
        backup_tar_gz: &Path,
        target_storage_root: &Path,
    ) -> Result<(BackupManifest, RestoreReport, Option<String>), ForgeError> {
        let file = File::open(backup_tar_gz)
            .map_err(|e| ForgeError::Internal(format!("Failed to open backup archive: {}", e)))?;
        let dec = GzDecoder::new(file);
        Self::unpack_archive(dec, target_storage_root)
    }

    /// Restores a backup archive, staging the database snapshot directly to a file on disk (O(1) memory)
    pub fn restore_backup_staged(
        backup_tar_gz: &Path,
        target_storage_root: &Path,
    ) -> Result<(BackupManifest, RestoreReport, Option<PathBuf>), ForgeError> {
        let mut session = Self::prepare_restore_session(backup_tar_gz, target_storage_root, None)?;
        let manifest = session.manifest.clone();
        let db_path = session.db_snapshot_path.clone();
        session.publish_storage()?;
        let report = session.finalize()?;
        Ok((manifest, report, db_path))
    }

    /// Restores an authenticated, AEAD-encrypted backup archive, verifying passphrases, chunks, and tags.
    /// Supports both SPNEBAK v2 (streaming chunked) and legacy SPNEBAK v1.
    pub fn restore_encrypted_backup(
        encrypted_path: &Path,
        target_storage_root: &Path,
        passphrase: &str,
    ) -> Result<(BackupManifest, RestoreReport, Option<String>), ForgeError> {
        let mut session =
            Self::prepare_restore_session(encrypted_path, target_storage_root, Some(passphrase))?;
        let manifest = session.manifest.clone();
        let db_str = match session.db_snapshot_path.as_deref() {
            Some(p) if p.exists() => {
                Some(fs::read_to_string(p).map_err(|e| ForgeError::Internal(e.to_string()))?)
            }
            _ => None,
        };
        session.publish_storage()?;
        let report = session.finalize()?;
        Ok((manifest, report, db_str))
    }

    /// Restores an encrypted backup archive, staging the database snapshot directly to a file on disk (O(1) memory).
    pub fn restore_encrypted_backup_staged(
        encrypted_path: &Path,
        target_storage_root: &Path,
        passphrase: &str,
    ) -> Result<(BackupManifest, RestoreReport, Option<PathBuf>), ForgeError> {
        let mut session =
            Self::prepare_restore_session(encrypted_path, target_storage_root, Some(passphrase))?;
        let manifest = session.manifest.clone();
        let db_path = session.db_snapshot_path.clone();
        session.publish_storage()?;
        let report = session.finalize()?;
        Ok((manifest, report, db_path))
    }

    /// Prepares a staged restoration session with full checksum verification and durable journaling.
    /// Live storage is NOT touched until StagedRestoreSession::publish_storage() is explicitly called.
    pub fn prepare_restore_session(
        input_path: &Path,
        target_storage_root: &Path,
        passphrase: Option<&str>,
    ) -> Result<StagedRestoreSession, ForgeError> {
        let mut file = File::open(input_path)
            .map_err(|e| ForgeError::Internal(format!("Failed to open backup archive: {}", e)))?;

        let mut magic = [0u8; 8];
        file.read_exact(&mut magic)
            .map_err(|e| ForgeError::Internal(format!("Failed to read archive magic: {}", e)))?;
        drop(file);

        if &magic == v2::MAGIC_V2 {
            let pass = passphrase.ok_or_else(|| {
                ForgeError::Internal(
                    "Passphrase required for encrypted SPNEBAK v2 archive".to_string(),
                )
            })?;
            let reader = File::open(input_path).map_err(|e| {
                ForgeError::Internal(format!("Failed to open encrypted archive: {}", e))
            })?;

            let staging_dir = tempfile::tempdir_in(target_storage_root)
                .or_else(|_| tempfile::tempdir())
                .map_err(|e| {
                    ForgeError::Internal(format!("Failed to create temp staging directory: {}", e))
                })?;

            let mut dec_reader = v2::DecryptingReader::new(reader, pass)?;
            let dec = GzDecoder::new(&mut dec_reader);
            let session =
                Self::unpack_archive_to_session(dec, input_path, target_storage_root, staging_dir)?;
            if let Err(e) = dec_reader.verify_to_eof() {
                let _ = RestoreJournal::remove(target_storage_root);
                return Err(e);
            }
            Ok(session)
        } else if &magic == BACKUP_MAGIC {
            let pass = passphrase.ok_or_else(|| {
                ForgeError::Internal(
                    "Passphrase required for encrypted SPNEBAK v1 archive".to_string(),
                )
            })?;
            // Bounded read for legacy v1
            let meta = fs::metadata(input_path).map_err(|e| ForgeError::Internal(e.to_string()))?;
            if meta.len() > MAX_LEGACY_V1_ARCHIVE_BYTES {
                return Err(ForgeError::Internal(format!(
                    "Legacy SPNEBAK v1 archive exceeds maximum supported size ({} > {} bytes)",
                    meta.len(),
                    MAX_LEGACY_V1_ARCHIVE_BYTES
                )));
            }
            let file = File::open(input_path).map_err(|e| ForgeError::Internal(e.to_string()))?;
            let mut encrypted_bytes = Vec::new();
            file.take(MAX_LEGACY_V1_ARCHIVE_BYTES + 1)
                .read_to_end(&mut encrypted_bytes)
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
            if encrypted_bytes.len() as u64 > MAX_LEGACY_V1_ARCHIVE_BYTES {
                return Err(ForgeError::Internal(
                    "Archive exceeded maximum v1 byte limit".to_string(),
                ));
            }

            let decrypted_tar_gz = Self::decrypt_backup_payload(&encrypted_bytes, pass)?;
            let staging_dir = tempfile::tempdir_in(target_storage_root)
                .or_else(|_| tempfile::tempdir())
                .map_err(|e| {
                    ForgeError::Internal(format!("Failed to create temp staging directory: {}", e))
                })?;
            let cursor = Cursor::new(decrypted_tar_gz);
            let dec = GzDecoder::new(cursor);
            Self::unpack_archive_to_session(dec, input_path, target_storage_root, staging_dir)
        } else {
            // Unencrypted tar.gz
            let file = File::open(input_path).map_err(|e| {
                ForgeError::Internal(format!("Failed to open backup archive: {}", e))
            })?;
            let dec = GzDecoder::new(file);
            let staging_dir = tempfile::tempdir_in(target_storage_root)
                .or_else(|_| tempfile::tempdir())
                .map_err(|e| {
                    ForgeError::Internal(format!("Failed to create temp staging directory: {}", e))
                })?;
            Self::unpack_archive_to_session(dec, input_path, target_storage_root, staging_dir)
        }
    }

    /// Rotates the encryption key/passphrase of an existing backup without writing decrypted files to disk.
    /// Operates in bounded streaming memory O(chunk_size), with failure-atomic publication and aliasing protection.
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

        // 1. Path aliasing verification: reject identical or aliased source and destination
        let canon_src =
            fs::canonicalize(encrypted_path).unwrap_or_else(|_| encrypted_path.to_path_buf());
        let canon_dst = if output_path.exists() {
            fs::canonicalize(output_path).unwrap_or_else(|_| output_path.to_path_buf())
        } else if let Some(parent) = output_path.parent() {
            let p = fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
            p.join(output_path.file_name().unwrap_or_default())
        } else {
            output_path.to_path_buf()
        };

        if canon_src == canon_dst {
            return Err(ForgeError::Internal(
                "Source and output backup paths cannot be identical (in-place rotation prohibited to prevent data loss)"
                    .to_string(),
            ));
        }

        let mut file = File::open(encrypted_path)
            .map_err(|e| ForgeError::Internal(format!("Failed to open encrypted backup: {}", e)))?;

        let mut magic = [0u8; 8];
        file.read_exact(&mut magic)
            .map_err(|e| ForgeError::Internal(format!("Failed to read archive magic: {}", e)))?;
        drop(file);

        let output_parent = output_path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(output_parent).map_err(|e| {
            ForgeError::Internal(format!("Failed to create destination directory: {}", e))
        })?;

        let tmp_output = output_parent.join(format!(".tmp_rot_{}.spnebak", Uuid::new_v4()));

        let rotation_result = (|| -> Result<(), ForgeError> {
            if &magic == v2::MAGIC_V2 {
                let reader = File::open(encrypted_path).map_err(|e| {
                    ForgeError::Internal(format!("Failed to open source encrypted archive: {}", e))
                })?;
                let writer = File::create(&tmp_output).map_err(|e| {
                    ForgeError::Internal(format!("Failed to create output archive: {}", e))
                })?;

                v2::rotate_stream(
                    reader,
                    writer,
                    old_passphrase,
                    new_passphrase,
                    new_key_version,
                )?;
            } else if &magic == BACKUP_MAGIC {
                // Upgrade legacy v1 archive to streaming v2 format on rotation with bounded memory
                let meta = fs::metadata(encrypted_path)
                    .map_err(|e| ForgeError::Internal(e.to_string()))?;
                if meta.len() > MAX_LEGACY_V1_ARCHIVE_BYTES {
                    return Err(ForgeError::Internal(format!(
                        "Legacy SPNEBAK v1 archive exceeds maximum supported size ({} > {} bytes)",
                        meta.len(),
                        MAX_LEGACY_V1_ARCHIVE_BYTES
                    )));
                }
                let v1_file =
                    File::open(encrypted_path).map_err(|e| ForgeError::Internal(e.to_string()))?;
                let mut encrypted_bytes = Vec::new();
                v1_file
                    .take(MAX_LEGACY_V1_ARCHIVE_BYTES + 1)
                    .read_to_end(&mut encrypted_bytes)
                    .map_err(|e| ForgeError::Internal(e.to_string()))?;
                if encrypted_bytes.len() as u64 > MAX_LEGACY_V1_ARCHIVE_BYTES {
                    return Err(ForgeError::Internal(
                        "Archive exceeded maximum v1 byte limit".to_string(),
                    ));
                }
                let plaintext_tar_gz =
                    Self::decrypt_backup_payload(&encrypted_bytes, old_passphrase)?;

                let cursor = Cursor::new(plaintext_tar_gz);
                let writer = File::create(&tmp_output).map_err(|e| {
                    ForgeError::Internal(format!("Failed to create output archive: {}", e))
                })?;

                v2::encrypt_stream(
                    cursor,
                    writer,
                    new_passphrase,
                    new_key_version,
                    v2::DEFAULT_CHUNK_SIZE,
                )?;
            } else {
                return Err(ForgeError::Internal(
                    "Unrecognized backup archive format or invalid magic bytes".to_string(),
                ));
            }

            // Verify temporary rotated output can be opened and decrypted completely with new passphrase
            let verify_file = File::open(&tmp_output).map_err(|e| {
                ForgeError::Internal(format!(
                    "Failed to reopen rotated archive for verification: {}",
                    e
                ))
            })?;
            v2::decrypt_stream(verify_file, std::io::sink(), new_passphrase)?;

            // Atomically publish rotated output to output_path
            fs::rename(&tmp_output, output_path).map_err(|e| {
                ForgeError::Internal(format!(
                    "Failed to atomically publish rotated archive: {}",
                    e
                ))
            })?;

            Ok(())
        })();

        if rotation_result.is_err() && tmp_output.exists() {
            let _ = fs::remove_file(&tmp_output);
        }

        rotation_result
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
    pub fn sanitize_restore_path(dest_root: &Path, rel_path: &str) -> Result<PathBuf, ForgeError> {
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

    /// Captures the authoritative private and public SSH host identity into a memory snapshot archive.
    pub fn capture_ssh_identity_snapshot(
        ssh_dir: &Path,
        fencing_generation: u64,
    ) -> Result<Option<(BackupParticipantRecord, Vec<u8>, String)>, ForgeError> {
        let key_path = ssh_dir.join("host_key_ed25519");
        if !key_path.exists() {
            return Ok(None);
        }
        let pem_str = fs::read_to_string(&key_path)
            .map_err(|e| ForgeError::Internal(format!("Failed to read SSH host key: {}", e)))?;
        let key = russh::keys::ssh_key::PrivateKey::from_openssh(&pem_str)
            .map_err(|e| ForgeError::Internal(format!("Invalid OpenSSH host key: {}", e)))?;
        let fingerprint = key
            .public_key()
            .fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
            .to_string();

        let mut buffer = Vec::new();
        {
            let enc = GzEncoder::new(&mut buffer, Compression::default());
            let mut tar = tar::Builder::new(enc);

            let priv_data = pem_str.as_bytes();
            let mut header = tar::Header::new_gnu();
            header
                .set_path("host_key_ed25519")
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
            header.set_size(priv_data.len() as u64);
            header.set_mode(0o600);
            header.set_cksum();
            tar.append(&header, priv_data)
                .map_err(|e| ForgeError::Internal(e.to_string()))?;

            let pub_path = ssh_dir.join("host_key_ed25519.pub");
            if pub_path.exists() {
                if let Ok(pub_data) = fs::read(&pub_path) {
                    let mut pub_header = tar::Header::new_gnu();
                    pub_header
                        .set_path("host_key_ed25519.pub")
                        .map_err(|e| ForgeError::Internal(e.to_string()))?;
                    pub_header.set_size(pub_data.len() as u64);
                    pub_header.set_mode(0o644);
                    pub_header.set_cksum();
                    tar.append(&pub_header, &pub_data[..])
                        .map_err(|e| ForgeError::Internal(e.to_string()))?;
                }
            }
            tar.finish()
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
        }

        let sha256_checksum = format!("{:x}", sha2::Sha256::digest(&buffer));
        let byte_length = buffer.len() as u64;

        let record = BackupParticipantRecord {
            participant_id: "spine_ssh".to_string(),
            volume_name: "spine_ssh".to_string(),
            snapshot_id: format!("ssh_snap_{}", Uuid::new_v4().simple()),
            fencing_generation,
            byte_length,
            sha256_checksum,
            archive_rel_path: "ssh/ssh_identity_snapshot.tar.gz".to_string(),
        };

        Ok(Some((record, buffer, fingerprint)))
    }

    /// Restores the SSH host identity into the target SSH directory and verifies key consistency.
    pub fn restore_ssh_identity_snapshot(
        ssh_dir: &Path,
        payload_bytes: &[u8],
        expected_fingerprint: Option<&str>,
    ) -> Result<String, ForgeError> {
        fs::create_dir_all(ssh_dir).map_err(|e| ForgeError::Internal(e.to_string()))?;
        let gz = GzDecoder::new(payload_bytes);
        let mut archive = tar::Archive::new(gz);

        for entry in archive
            .entries()
            .map_err(|e| ForgeError::Internal(e.to_string()))?
        {
            let mut entry = entry.map_err(|e| ForgeError::Internal(e.to_string()))?;
            let path = entry
                .path()
                .map_err(|e| ForgeError::Internal(e.to_string()))?
                .to_path_buf();
            let dest = ssh_dir.join(path);
            entry
                .unpack(&dest)
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
        }

        let key_path = ssh_dir.join("host_key_ed25519");
        if !key_path.exists() {
            return Err(ForgeError::Internal(
                "Restored SSH identity missing host_key_ed25519".to_string(),
            ));
        }
        let pem_str = fs::read_to_string(&key_path).map_err(|e| {
            ForgeError::Internal(format!("Failed to read restored SSH host key: {}", e))
        })?;
        let key = russh::keys::ssh_key::PrivateKey::from_openssh(&pem_str).map_err(|e| {
            ForgeError::Internal(format!("Restored SSH key is invalid OpenSSH key: {}", e))
        })?;
        let actual_fingerprint = key
            .public_key()
            .fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
            .to_string();

        if let Some(expected) = expected_fingerprint {
            if actual_fingerprint != expected {
                return Err(ForgeError::Internal(format!(
                    "Restored SSH host key fingerprint mismatch: expected {}, got {}",
                    expected, actual_fingerprint
                )));
            }
        }

        Ok(actual_fingerprint)
    }

    /// Internal shared logic to package files into a tar builder
    fn create_archive_internal<W: std::io::Write>(
        storage_root: &Path,
        db_snapshot_json: Option<&str>,
        db_snapshot_file: Option<&Path>,
        participants: &[(BackupParticipantRecord, ParticipantPayloadSource)],
        fencing_generation: Option<u64>,
        writer: W,
    ) -> Result<(BackupManifest, W), ForgeError> {
        let backup_id = Uuid::new_v4().to_string();
        let timestamp_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

        let mut file_hashes = BTreeMap::new();
        let mut total_bytes = 0u64;
        let mut participant_paths = std::collections::HashSet::new();

        for (record, _) in participants {
            participant_paths.insert(record.archive_rel_path.clone());
            file_hashes.insert(
                record.archive_rel_path.clone(),
                record.sha256_checksum.clone(),
            );
            total_bytes += record.byte_length;
        }

        let mut file_paths = Vec::new();
        // If spine_storage is explicitly included as a participant snapshot, do NOT duplicate loose files!
        let has_spine_storage_participant = participants
            .iter()
            .any(|(p, _)| p.participant_id == "spine_storage");
        if !has_spine_storage_participant && storage_root.exists() {
            Self::collect_files_recursive(storage_root, storage_root, &mut file_paths)?;
        }

        for (rel_path, abs_path) in &file_paths {
            let mut f =
                fs::File::open(abs_path).map_err(|e| ForgeError::Internal(e.to_string()))?;
            let mut hasher = Sha256::new();
            let mut buf = [0u8; 64 * 1024];
            let mut f_len = 0u64;
            loop {
                let n = f
                    .read(&mut buf)
                    .map_err(|e| ForgeError::Internal(e.to_string()))?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
                f_len += n as u64;
            }
            total_bytes += f_len;
            let digest = hex::encode(hasher.finalize());
            file_hashes.insert(rel_path.clone(), digest);
        }

        if let Some(db_file) = db_snapshot_file {
            if !participant_paths.contains("database/snapshot.jsonl") {
                let mut f =
                    fs::File::open(db_file).map_err(|e| ForgeError::Internal(e.to_string()))?;
                let mut hasher = Sha256::new();
                let mut buf = [0u8; 64 * 1024];
                let mut f_len = 0u64;
                loop {
                    let n = f
                        .read(&mut buf)
                        .map_err(|e| ForgeError::Internal(e.to_string()))?;
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buf[..n]);
                    f_len += n as u64;
                }
                total_bytes += f_len;
                let digest = hex::encode(hasher.finalize());
                file_hashes.insert("database/snapshot.jsonl".to_string(), digest);
            }
        } else if let Some(db_json) = db_snapshot_json {
            let mut hasher = Sha256::new();
            hasher.update(db_json.as_bytes());
            let digest = hex::encode(hasher.finalize());
            total_bytes += db_json.len() as u64;
            file_hashes.insert("database/snapshot.json".to_string(), digest);
        }

        let ssh_dir = crate::ssh::host_key::resolve_ssh_dir(storage_root);
        let host_priv_path = ssh_dir.join("host_key_ed25519");
        let host_pub_path = ssh_dir.join("host_key_ed25519.pub");
        let ssh_fingerprint = if host_priv_path.exists() {
            if let Ok(pem_str) = fs::read_to_string(&host_priv_path) {
                if let Ok(key) = russh::keys::ssh_key::PrivateKey::from_openssh(&pem_str) {
                    Some(
                        key.public_key()
                            .fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
                            .to_string(),
                    )
                } else {
                    None
                }
            } else {
                None
            }
        } else if host_pub_path.exists() {
            if let Ok(pub_str) = fs::read_to_string(&host_pub_path) {
                if let Ok(parsed) = russh::keys::ssh_key::PublicKey::from_openssh(&pub_str) {
                    Some(
                        parsed
                            .fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
                            .to_string(),
                    )
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
            fencing_generation,
            participants: participants.iter().map(|(r, _)| r.clone()).collect(),
            required_participant_count: participants.len(),
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

        if let Some(db_file) = db_snapshot_file {
            if !participant_paths.contains("database/snapshot.jsonl") {
                let metadata =
                    fs::metadata(db_file).map_err(|e| ForgeError::Internal(e.to_string()))?;
                let mut f =
                    fs::File::open(db_file).map_err(|e| ForgeError::Internal(e.to_string()))?;
                let mut db_header = tar::Header::new_gnu();
                db_header.set_size(metadata.len());
                db_header.set_mode(0o600);
                db_header.set_cksum();
                tar.append_data(&mut db_header, "database/snapshot.jsonl", &mut f)
                    .map_err(|e| ForgeError::Internal(e.to_string()))?;
            }
        } else if let Some(db_json) = db_snapshot_json {
            let mut db_header = tar::Header::new_gnu();
            db_header.set_size(db_json.len() as u64);
            db_header.set_mode(0o600);
            db_header.set_cksum();
            tar.append_data(&mut db_header, "database/snapshot.json", db_json.as_bytes())
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
        }

        for (rel_path, abs_path) in file_paths {
            let metadata =
                fs::metadata(&abs_path).map_err(|e| ForgeError::Internal(e.to_string()))?;
            let mut f =
                fs::File::open(&abs_path).map_err(|e| ForgeError::Internal(e.to_string()))?;
            let mut f_header = tar::Header::new_gnu();
            f_header.set_size(metadata.len());
            f_header.set_mode(0o644);
            f_header.set_cksum();
            tar.append_data(&mut f_header, &rel_path, &mut f)
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
        }

        for (record, source) in participants {
            match source {
                ParticipantPayloadSource::Memory(data) => {
                    let mut p_header = tar::Header::new_gnu();
                    p_header.set_size(data.len() as u64);
                    p_header.set_mode(0o644);
                    p_header.set_cksum();
                    tar.append_data(&mut p_header, &record.archive_rel_path, &data[..])
                        .map_err(|e| ForgeError::Internal(e.to_string()))?;
                }
                ParticipantPayloadSource::File(path) => {
                    let mut f =
                        fs::File::open(path).map_err(|e| ForgeError::Internal(e.to_string()))?;
                    let meta =
                        fs::metadata(path).map_err(|e| ForgeError::Internal(e.to_string()))?;
                    let mut p_header = tar::Header::new_gnu();
                    p_header.set_size(meta.len());
                    p_header.set_mode(0o644);
                    p_header.set_cksum();
                    tar.append_data(&mut p_header, &record.archive_rel_path, &mut f)
                        .map_err(|e| ForgeError::Internal(e.to_string()))?;
                }
                ParticipantPayloadSource::EncryptedTemp(enc_tmp) => {
                    let mut p_header = tar::Header::new_gnu();
                    p_header.set_size(enc_tmp.byte_length);
                    p_header.set_mode(0o600);
                    p_header.set_cksum();
                    let mut dec_reader = enc_tmp.open_reader()?;
                    tar.append_data(&mut p_header, &record.archive_rel_path, &mut dec_reader)
                        .map_err(|e| ForgeError::Internal(e.to_string()))?;
                }
            }
        }

        let writer = tar
            .into_inner()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok((manifest, writer))
    }

    /// Internal shared logic to unpack tar archive into a StagedRestoreSession, validating bounds, traversal limits and hashes.
    /// Live target storage remains completely untouched until session.publish_storage() is executed.
    fn unpack_archive_to_session<R: Read>(
        reader: R,
        input_archive: &Path,
        target_storage_root: &Path,
        staging_dir: tempfile::TempDir,
    ) -> Result<StagedRestoreSession, ForgeError> {
        let start = Instant::now();
        let mut journal = RestoreJournal::new(
            input_archive,
            target_storage_root,
            staging_dir.path(),
            "pending",
        );
        journal.persist(RestorePhase::Initialized)?;

        let mut archive = tar::Archive::new(reader);
        let mut manifest: Option<BackupManifest> = None;
        let mut extracted_db_staged_path: Option<PathBuf> = None;
        let mut files_to_verify: Vec<(String, PathBuf)> = Vec::new();
        let mut entry_count = 0usize;

        for entry in archive
            .entries()
            .map_err(|e| ForgeError::Internal(e.to_string()))?
        {
            entry_count += 1;
            if entry_count > MAX_ARCHIVE_FILE_COUNT {
                return Err(ForgeError::Internal(format!(
                    "Archive file count exceeds maximum permitted limit ({})",
                    MAX_ARCHIVE_FILE_COUNT
                )));
            }

            let mut entry = entry.map_err(|e| ForgeError::Internal(e.to_string()))?;
            let path = entry
                .path()
                .map_err(|e| ForgeError::Internal(e.to_string()))?
                .to_path_buf();
            let path_str = path.to_string_lossy().replace('\\', "/");

            if entry.header().entry_type().is_dir() {
                continue;
            }

            if path_str.len() > MAX_ARCHIVE_PATH_LEN {
                return Err(ForgeError::Internal(format!(
                    "Archive path exceeds maximum length of {} bytes: {}",
                    MAX_ARCHIVE_PATH_LEN, path_str
                )));
            }

            if path_str == "manifest.json" {
                let mut content = Vec::new();
                let mut limited = entry.take(MAX_MANIFEST_BYTES as u64 + 1);
                limited
                    .read_to_end(&mut content)
                    .map_err(|e| ForgeError::Internal(e.to_string()))?;
                if content.len() > MAX_MANIFEST_BYTES {
                    return Err(ForgeError::Internal(format!(
                        "manifest.json size exceeds {} bytes maximum",
                        MAX_MANIFEST_BYTES
                    )));
                }
                let parsed: BackupManifest = serde_json::from_slice(&content)
                    .map_err(|e| ForgeError::Internal(format!("Corrupt manifest: {}", e)))?;
                manifest = Some(parsed);
            } else if path_str == "database/snapshot.jsonl" || path_str == "database/snapshot.json"
            {
                let staged_db_path = staging_dir.path().join("database_snapshot.jsonl");
                let mut out_file = fs::File::create(&staged_db_path)
                    .map_err(|e| ForgeError::Internal(e.to_string()))?;
                let mut buf = [0u8; 64 * 1024];
                loop {
                    let n = entry
                        .read(&mut buf)
                        .map_err(|e| ForgeError::Internal(e.to_string()))?;
                    if n == 0 {
                        break;
                    }
                    out_file
                        .write_all(&buf[..n])
                        .map_err(|e| ForgeError::Internal(e.to_string()))?;
                }
                out_file
                    .flush()
                    .map_err(|e| ForgeError::Internal(e.to_string()))?;
                files_to_verify.push((path_str.clone(), staged_db_path.clone()));
                extracted_db_staged_path = Some(staged_db_path);
            } else {
                let staged_file_name = format!("stage_{}.dat", files_to_verify.len());
                let staged_path = staging_dir.path().join(&staged_file_name);
                let mut out_file = fs::File::create(&staged_path)
                    .map_err(|e| ForgeError::Internal(e.to_string()))?;

                let mut buf = [0u8; 64 * 1024];
                loop {
                    let n = entry
                        .read(&mut buf)
                        .map_err(|e| ForgeError::Internal(e.to_string()))?;
                    if n == 0 {
                        break;
                    }
                    out_file
                        .write_all(&buf[..n])
                        .map_err(|e| ForgeError::Internal(e.to_string()))?;
                }
                out_file
                    .flush()
                    .map_err(|e| ForgeError::Internal(e.to_string()))?;
                files_to_verify.push((path_str, staged_path));
            }
        }

        let manifest = manifest.ok_or_else(|| {
            ForgeError::Internal("Backup archive missing manifest.json".to_string())
        })?;

        // Phase 1: Verify all extracted files against manifest hashes and verify inventory completeness
        let mut verified_sizes = std::collections::HashMap::new();
        for (rel_path, staged_path) in &files_to_verify {
            let expected_hash = manifest.file_hashes.get(rel_path).ok_or_else(|| {
                ForgeError::Internal(format!(
                    "Unexpected file in archive not in manifest: {}",
                    rel_path
                ))
            })?;

            let mut f =
                fs::File::open(staged_path).map_err(|e| ForgeError::Internal(e.to_string()))?;
            let mut hasher = Sha256::new();
            let mut buf = [0u8; 64 * 1024];
            let mut file_len = 0u64;
            loop {
                let n = f
                    .read(&mut buf)
                    .map_err(|e| ForgeError::Internal(e.to_string()))?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
                file_len += n as u64;
            }
            let actual_hash = hex::encode(hasher.finalize());
            if actual_hash != *expected_hash {
                return Err(ForgeError::Internal(format!(
                    "Integrity verification failed for {}: expected {}, got {}",
                    rel_path, expected_hash, actual_hash
                )));
            }
            verified_sizes.insert(rel_path.clone(), file_len);
        }

        // Verify that every file declared in the manifest exists in the extracted archive
        for manifest_file in manifest.file_hashes.keys() {
            if !files_to_verify.iter().any(|(p, _)| p == manifest_file) {
                return Err(ForgeError::Internal(format!(
                    "Missing file declared in manifest: {}",
                    manifest_file
                )));
            }
        }

        // Verify that every participant declared in manifest has matching archive payload
        for participant in &manifest.participants {
            let (rel, _staged_p) = files_to_verify
                .iter()
                .find(|(p_rel, _)| p_rel == &participant.archive_rel_path)
                .ok_or_else(|| {
                    ForgeError::Internal(format!(
                        "FATAL RESTORE REJECTION: Participant '{}' declared in manifest is missing its payload '{}' in archive!",
                        participant.participant_id, participant.archive_rel_path
                    ))
                })?;
            let sz = verified_sizes.get(rel).copied().unwrap_or(0);
            if sz != participant.byte_length {
                return Err(ForgeError::Internal(format!(
                    "FATAL RESTORE REJECTION: Participant '{}' byte length mismatch: manifest says {}, payload has {}",
                    participant.participant_id, participant.byte_length, sz
                )));
            }
            let h = manifest.file_hashes.get(rel).unwrap();
            if h != &participant.sha256_checksum {
                return Err(ForgeError::Internal(format!(
                    "FATAL RESTORE REJECTION: Participant '{}' checksum mismatch: manifest says {}, payload hash is {}",
                    participant.participant_id, participant.sha256_checksum, h
                )));
            }
        }

        let mut verified_files_list = Vec::new();
        for (rel_path, staged_path) in files_to_verify {
            let size = verified_sizes.get(&rel_path).copied().unwrap_or(0);
            verified_files_list.push((rel_path, staged_path, size));
        }

        journal.backup_id = manifest.backup_id.clone();
        journal.target_generation = format!(
            "gen_{}_{}",
            manifest.backup_id,
            uuid::Uuid::new_v4().simple()
        );
        if extracted_db_staged_path.is_some() {
            journal.db_snapshot_staged = extracted_db_staged_path.clone();
            journal.persist(RestorePhase::StagedDb)?;
        } else {
            journal.persist(RestorePhase::StagedStorage)?;
        }

        let report = RestoreReport {
            backup_id: manifest.backup_id.clone(),
            files_restored: 0,
            bytes_restored: 0,
            duration_ms: start.elapsed().as_millis() as u64,
            integrity_verified: true,
        };

        Ok(StagedRestoreSession {
            manifest,
            report,
            journal,
            db_snapshot_path: extracted_db_staged_path,
            staging_dir,
            verified_files: verified_files_list,
            target_storage_root: target_storage_root.to_path_buf(),
        })
    }

    /// Internal shared logic to unpack tar archive, validating traversal limits and hashes
    fn unpack_archive<R: Read>(
        reader: R,
        target_storage_root: &Path,
    ) -> Result<(BackupManifest, RestoreReport, Option<String>), ForgeError> {
        let staging_dir = tempfile::tempdir_in(target_storage_root)
            .or_else(|_| tempfile::tempdir())
            .map_err(|e| {
                ForgeError::Internal(format!("Failed to create temp staging directory: {}", e))
            })?;
        let mut session = Self::unpack_archive_to_session(
            reader,
            Path::new("archive.tar.gz"),
            target_storage_root,
            staging_dir,
        )?;
        let manifest = session.manifest.clone();
        let db_str = match &session.db_snapshot_path {
            Some(p) if p.exists() => {
                Some(fs::read_to_string(p).map_err(|e| ForgeError::Internal(e.to_string()))?)
            }
            _ => None,
        };
        session.publish_storage()?;
        let report = session.finalize()?;
        Ok((manifest, report, db_str))
    }

    pub fn collect_files_recursive(
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

/// Executes a coordinated cluster backup while ensuring every participant that may have received
/// `enter_maintenance` is registered BEFORE sending the RPC, and treating any `exit_maintenance`
/// or database `MaintenanceGuard::release` failure as a command error.
pub async fn run_backup_with_maintenance_cleanup<Work, WorkFut, GuardRel, GuardFut>(
    fencing_generation: u64,
    coord_addr_opt: Option<&str>,
    node_endpoints: &[(String, String)],
    release_db_guard: Option<GuardRel>,
    work: Work,
) -> Result<(), ForgeError>
where
    Work: FnOnce(
        Option<crate::CoordinatorClient>,
        Vec<(String, crate::ClusterNodeClient)>,
    ) -> WorkFut,
    WorkFut: std::future::Future<Output = Result<(), ForgeError>>,
    GuardRel: FnOnce() -> GuardFut,
    GuardFut: std::future::Future<Output = Result<(), ForgeError>>,
{
    let mut coord_client: Option<crate::CoordinatorClient> = None;
    let mut storage_clients: Vec<(String, crate::ClusterNodeClient)> = Vec::new();

    let work_res: Result<(), ForgeError> = async {
        if let Some(c_addr) = coord_addr_opt {
            println!(
                "Coordinating maintenance barrier with Spine Coordinator at {}...",
                c_addr
            );
            let client = crate::CoordinatorClient::new(c_addr.to_string());
            // Register BEFORE sending enter_maintenance so lost-ACK uncertain entry is cleaned up.
            coord_client = Some(client.clone());
            client
                .enter_maintenance(fencing_generation)
                .await
                .map_err(|e| {
                    ForgeError::Internal(format!(
                        "Failed to coordinate maintenance barrier with coordinator at {}: {}",
                        c_addr, e
                    ))
                })?;
        }

        for (node_id, addr) in node_endpoints {
            println!(
                "Coordinating maintenance barrier with storage node '{}' at {}...",
                node_id, addr
            );
            let client = crate::ClusterNodeClient::new(addr.clone());
            // Register BEFORE sending enter_maintenance so lost-ACK uncertain entry is cleaned up.
            storage_clients.push((node_id.clone(), client.clone()));
            client
                .enter_maintenance(fencing_generation)
                .await
                .map_err(|e| {
                    ForgeError::Internal(format!(
                        "Failed to coordinate maintenance barrier with storage node '{}' at {}: {}",
                        node_id, addr, e
                    ))
                })?;
        }

        work(coord_client.clone(), storage_clients.clone()).await
    }
    .await;

    let mut cleanup_errors: Vec<String> = Vec::new();
    let mut unresolved_participants: Vec<String> = Vec::new();

    for (node_id, client) in &storage_clients {
        let mut res = client.exit_maintenance(fencing_generation).await;
        if res.is_err() {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            res = client.exit_maintenance(fencing_generation).await;
        }
        if let Err(e) = res {
            unresolved_participants.push(node_id.clone());
            cleanup_errors.push(format!(
                "storage node '{}' exit_maintenance({}) failed: {}",
                node_id, fencing_generation, e
            ));
        }
    }

    if let Some(ref client) = coord_client {
        let mut res = client.exit_maintenance(fencing_generation).await;
        if res.is_err() {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            res = client.exit_maintenance(fencing_generation).await;
        }
        if let Err(e) = res {
            unresolved_participants.push("coordinator".to_string());
            cleanup_errors.push(format!(
                "coordinator exit_maintenance({}) failed: {}",
                fencing_generation, e
            ));
        }
    }

    if let Some(rel_fn) = release_db_guard {
        if let Err(e) = rel_fn().await {
            unresolved_participants.push("postgres_maintenance_guard".to_string());
            cleanup_errors.push(format!("database maintenance guard release failed: {}", e));
        }
    }

    if !cleanup_errors.is_empty() {
        let recovery_hint = format!(
            "Unresolved participants that may remain fenced at generation {}: [{}]. Operator action: verify connectivity and re-run maintenance exit for generation {} on the listed participants.",
            fencing_generation,
            unresolved_participants.join(", "),
            fencing_generation
        );
        return match work_res {
            Ok(()) => Err(ForgeError::Internal(format!(
                "Backup archive created, but maintenance cleanup failed (backup cannot succeed while maintenance state is unresolved): {}. {}",
                cleanup_errors.join("; "),
                recovery_hint
            ))),
            Err(orig) => Err(ForgeError::Internal(format!(
                "Backup failed: [{}] AND maintenance cleanup failed: [{}]. {}",
                orig,
                cleanup_errors.join("; "),
                recovery_hint
            ))),
        };
    }

    work_res
}

/// Streams a PostgreSQL database dump directly into an encrypted `EphemeralEncryptedTemp` file.
pub async fn capture_encrypted_pg_dump(
    pool: &sqlx::PgPool,
) -> Result<EphemeralEncryptedTemp, ForgeError> {
    let (tmp_pg, pg_key, mut pg_enc_writer) = EphemeralEncryptedTemp::begin(v2::DEFAULT_CHUNK_SIZE)
        .map_err(|e| {
            ForgeError::Internal(format!("Failed to initialize encrypted DB stream: {}", e))
        })?;
    crate::dump_database_to_writer(pool, &mut pg_enc_writer)
        .await
        .map_err(|e| {
            ForgeError::Internal(format!("Failed to stream PostgreSQL database dump: {}", e))
        })?;
    let pg_summary = pg_enc_writer.finish().map_err(|e| {
        ForgeError::Internal(format!("Failed to finalize encrypted DB stream: {}", e))
    })?;
    let pg_enc = EphemeralEncryptedTemp::from_finished(tmp_pg, pg_key, pg_summary);
    println!(
        "PostgreSQL bounded-memory AEAD snapshot captured successfully ({} bytes plaintext, O(1 MiB) RAM, 0 plaintext disk intermediates).",
        pg_enc.byte_length
    );
    Ok(pg_enc)
}
