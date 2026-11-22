use anyhow::{bail, Context, Result};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::Digest;
use spine_cluster::fencing::OwnershipEpoch;
use spine_cluster::node::NodeCapacity;
use spine_cluster::protocol::{ParticipantAcknowledgement, VolumeSnapshotManifest};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use tar::{Archive, Builder};

fn compute_dir_size(path: &Path) -> u64 {
    let mut size = 0;
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                size += compute_dir_size(&p);
            } else if let Ok(meta) = entry.metadata() {
                size += meta.len();
            }
        }
    }
    size
}

fn count_files_recursive(path: &Path) -> usize {
    let mut count = 0;
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                count += count_files_recursive(&p);
            } else {
                count += 1;
            }
        }
    }
    count
}

pub trait StorageBackend: Send + Sync {
    fn repo_path(&self, repo_id: &str) -> PathBuf;
    fn get_capacity(&self) -> Result<NodeCapacity>;
    fn get_epoch(&self, repo_id: &str) -> Result<u64>;
    fn set_epoch(&self, repo_id: &str, epoch: u64) -> Result<()>;
    fn get_generation(&self, repo_id: &str) -> Result<u64>;
    fn set_generation(&self, repo_id: &str, generation: u64) -> Result<()>;

    #[allow(clippy::too_many_arguments)]
    fn stage_pack(
        &self,
        tx_id: &str,
        repo_id: &str,
        epoch: u64,
        pack_id: &str,
        pack_checksum: &str,
        pack_bytes: &[u8],
        index_bytes: &[u8],
    ) -> Result<()>;

    fn commit_pack(
        &self,
        tx_id: &str,
        repo_id: &str,
        epoch: u64,
        new_generation: u64,
    ) -> Result<()>;

    fn abort_pack(&self, tx_id: &str) -> Result<()>;

    fn ingest_replicated_pack(
        &self,
        repo_id: &str,
        pack_id: &str,
        pack_checksum: &str,
        pack_bytes: &[u8],
        index_bytes: &[u8],
    ) -> Result<()>;

    fn list_packs(&self, repo_id: &str) -> Result<Vec<String>>;
    fn read_pack(&self, repo_id: &str, pack_id: &str) -> Result<(Vec<u8>, Vec<u8>)>;
    fn verify_repository(&self, repo_id: &str) -> Result<bool>;

    fn store_chunk(&self, chunk_hash: &str, bytes: &[u8]) -> Result<()>;
    fn get_chunk(&self, chunk_hash: &str) -> Result<Option<Vec<u8>>>;
    fn store_manifest(&self, manifest_id: &str, bytes: &[u8]) -> Result<()>;
    fn get_manifest(&self, manifest_id: &str) -> Result<Option<Vec<u8>>>;

    fn enter_maintenance(&self, fencing_generation: u64) -> Result<()>;
    fn exit_maintenance(&self, fencing_generation: u64) -> Result<()>;
    fn get_maintenance_status(&self) -> (bool, u64, usize);
    fn node_id(&self) -> &str;
    fn capture_volume_snapshot(
        &self,
        snapshot_id: &str,
        fencing_generation: u64,
    ) -> Result<(VolumeSnapshotManifest, Vec<u8>)>;
    fn stage_volume_restore(
        &self,
        snapshot_id: &str,
        fencing_generation: u64,
        payload_tar_gz: &[u8],
    ) -> Result<ParticipantAcknowledgement> {
        self.stage_volume_restore_with_manifest(
            snapshot_id,
            fencing_generation,
            None,
            payload_tar_gz,
        )
    }
    fn stage_volume_restore_with_manifest(
        &self,
        snapshot_id: &str,
        fencing_generation: u64,
        manifest: Option<&VolumeSnapshotManifest>,
        payload_tar_gz: &[u8],
    ) -> Result<ParticipantAcknowledgement>;
    fn commit_volume_restore(&self, snapshot_id: &str, fencing_generation: u64) -> Result<()>;
    fn rollback_volume_restore(&self, snapshot_id: &str) -> Result<()>;
    fn finalize_volume_restore(&self, snapshot_id: &str) -> Result<()>;
    fn query_staged_volume_restore(
        &self,
        snapshot_id: &str,
    ) -> Result<Option<ParticipantAcknowledgement>>;
    fn verify_volume_integrity(&self) -> Result<(usize, usize, usize)>;
}

pub struct StorageWriterGuard {
    active_writers: Arc<AtomicUsize>,
}

impl Drop for StorageWriterGuard {
    fn drop(&mut self) {
        self.active_writers.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(windows)]
fn is_write_through_unsupported(err: &std::io::Error) -> bool {
    matches!(err.raw_os_error(), Some(650) | Some(50) | Some(1))
}

#[cfg(windows)]
fn win32_sync_file_handle(path: &Path) -> std::io::Result<()> {
    let file = fs::OpenOptions::new().write(true).open(path)?;
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

/// Atomically replaces `to` with `from` on Windows, propagating source/destination flush errors.
/// Falls back to non-write-through rename + destination `FlushFileBuffers` only when the volume
/// rejects `WRITE_THROUGH` IRPs (OS errors 650, 50, or 1).
#[cfg(windows)]
fn platform_atomic_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    win32_sync_file_handle(from)?;
    match platform_atomic_replace_inner(from, to, true) {
        Ok(()) => Ok(()),
        Err(err) if is_write_through_unsupported(&err) => {
            platform_atomic_replace_inner(from, to, false)?;
            win32_sync_file_handle(to)?;
            Ok(())
        }
        Err(err) => Err(err),
    }
}

#[cfg(unix)]
fn platform_atomic_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to)
}

pub fn safe_atomic_replace(from: &Path, to: &Path) -> Result<()> {
    #[cfg(unix)]
    let sync_dir = |p: &Path| -> Result<()> {
        if let Ok(p_file) = fs::File::open(p) {
            p_file
                .sync_all()
                .with_context(|| format!("Failed to sync directory {}", p.display()))?;
        }
        Ok(())
    };
    #[cfg(windows)]
    let sync_dir = |p: &Path| -> Result<()> {
        if !p.exists() {
            bail!("Directory does not exist for sync: {}", p.display());
        }
        Ok(())
    };

    platform_atomic_replace(from, to).with_context(|| {
        format!(
            "Failed atomic replace from {} to {}",
            from.display(),
            to.display()
        )
    })?;

    if !to.exists() || fs::metadata(to).is_err() {
        bail!(
            "Readback verification failed after replacing {}",
            to.display()
        );
    }

    if let Some(parent) = to.parent() {
        sync_dir(parent)?;
    }

    Ok(())
}

pub struct FileSystemStorageBackend {
    storage_root: PathBuf,
    total_capacity: u64,
    in_maintenance: Arc<AtomicBool>,
    fencing_generation: Arc<AtomicU64>,
    active_writers: Arc<AtomicUsize>,
    mutation_barrier: Arc<std::sync::RwLock<()>>,
    node_id: String,
}

impl FileSystemStorageBackend {
    pub fn new<P: AsRef<Path>>(storage_root: P, total_capacity: u64) -> Result<Self> {
        let root = storage_root.as_ref().to_path_buf();
        let repos_exist = root.join("repos").exists()
            && fs::read_dir(root.join("repos"))
                .map(|mut d| d.next().is_some())
                .unwrap_or(false);
        let chunks_exist = root.join("chunks").exists()
            && fs::read_dir(root.join("chunks"))
                .map(|mut d| d.next().is_some())
                .unwrap_or(false);
        let manifests_exist = root.join("manifests").exists()
            && fs::read_dir(root.join("manifests"))
                .map(|mut d| d.next().is_some())
                .unwrap_or(false);
        let has_existing_data = root.join(".spine_storage_generation").exists()
            || repos_exist
            || chunks_exist
            || manifests_exist;

        fs::create_dir_all(root.join("repos"))?;
        fs::create_dir_all(root.join("staging"))?;
        fs::create_dir_all(root.join("chunks"))?;
        fs::create_dir_all(root.join("manifests"))?;

        let fencing_path = root.join(".fencing_generation");
        let initial_gen = if fencing_path.exists() {
            let s = fs::read_to_string(&fencing_path).map_err(|e| {
                anyhow::anyhow!(
                    "Failed to read fencing generation from {}: {}",
                    fencing_path.display(),
                    e
                )
            })?;
            let gen = s.trim().parse::<u64>().map_err(|e| {
                anyhow::anyhow!(
                    "Corrupt fencing generation record in {}: {}",
                    fencing_path.display(),
                    e
                )
            })?;
            if gen == 0 {
                bail!("Invalid fencing generation 0 in {}", fencing_path.display());
            }
            gen
        } else if has_existing_data {
            bail!(
                "FATAL: Storage node root '{}' contains existing data, but fencing generation record '{}' is missing! Existing participants with missing fencing markers must fail closed.",
                root.display(),
                fencing_path.display()
            );
        } else {
            let _ = fs::write(&fencing_path, "1\n");
            1
        };

        Ok(Self {
            storage_root: root,
            total_capacity,
            in_maintenance: Arc::new(AtomicBool::new(false)),
            fencing_generation: Arc::new(AtomicU64::new(initial_gen)),
            active_writers: Arc::new(AtomicUsize::new(0)),
            mutation_barrier: Arc::new(std::sync::RwLock::new(())),
            node_id: "storage-node".to_string(),
        })
    }

    pub fn persist_fencing_generation(&self, gen: u64) -> Result<()> {
        let cur = self.fencing_generation.load(Ordering::SeqCst);
        if gen < cur {
            bail!(
                "Monotonicity violation: cannot decrement storage node fencing generation from {} to {}",
                cur,
                gen
            );
        }
        let target = std::cmp::max(cur, gen);
        let fencing_path = self.storage_root.join(".fencing_generation");
        let tmp = fencing_path.with_extension(format!("tmp.{}", uuid::Uuid::new_v4()));
        {
            let mut f = fs::File::create(&tmp)?;
            use std::io::Write;
            f.write_all(format!("{}\n", target).as_bytes())?;
            f.flush()?;
            f.sync_all()?;
        }
        if let Err(e) = safe_atomic_replace(&tmp, &fencing_path) {
            let _ = fs::remove_file(&tmp);
            bail!(
                "Failed to durably persist storage node fencing generation {}: {}",
                target,
                e
            );
        }
        self.fencing_generation.store(target, Ordering::SeqCst);
        Ok(())
    }

    pub fn with_node_id(mut self, node_id: String) -> Self {
        self.node_id = node_id;
        self
    }

    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    pub fn acquire_writer(&self, required_generation: Option<u64>) -> Result<StorageWriterGuard> {
        let _barrier = self
            .mutation_barrier
            .read()
            .map_err(|_| anyhow::anyhow!("Storage mutation barrier poisoned"))?;
        if self.in_maintenance.load(Ordering::SeqCst) {
            bail!("Storage node mutation rejected: node is currently in maintenance mode");
        }
        let cur_gen = self.fencing_generation.load(Ordering::SeqCst);
        if let Some(req_gen) = required_generation {
            if req_gen < cur_gen {
                bail!(
                    "Storage node mutation rejected: stale fencing generation {} (current {})",
                    req_gen,
                    cur_gen
                );
            }
        }
        self.active_writers.fetch_add(1, Ordering::SeqCst);
        Ok(StorageWriterGuard {
            active_writers: Arc::clone(&self.active_writers),
        })
    }

    fn staging_dir(&self, tx_id: &str) -> PathBuf {
        self.storage_root.join("staging").join(tx_id)
    }

    fn ensure_repo_dirs(&self, repo_id: &str) -> Result<PathBuf> {
        let repo_dir = self.repo_path(repo_id);
        let spine_dir = repo_dir.join(".spine");
        fs::create_dir_all(spine_dir.join("pack"))?;
        fs::create_dir_all(spine_dir.join("objects"))?;
        fs::create_dir_all(spine_dir.join("refs").join("heads"))?;

        let epoch_file = spine_dir.join("ownership_epoch");
        if !epoch_file.exists() {
            fs::write(&epoch_file, "1\n")?;
        }

        let gen_file = spine_dir.join("generation");
        if !gen_file.exists() {
            fs::write(&gen_file, "0\n")?;
        }

        Ok(spine_dir)
    }
}

impl StorageBackend for FileSystemStorageBackend {
    fn node_id(&self) -> &str {
        &self.node_id
    }

    fn repo_path(&self, repo_id: &str) -> PathBuf {
        self.storage_root.join("repos").join(repo_id)
    }

    fn get_capacity(&self) -> Result<NodeCapacity> {
        let used = if self.storage_root.exists() {
            compute_dir_size(&self.storage_root)
        } else {
            0
        };
        Ok(NodeCapacity::new(self.total_capacity, used, 0))
    }

    fn get_epoch(&self, repo_id: &str) -> Result<u64> {
        let spine_dir = self.ensure_repo_dirs(repo_id)?;
        let content = fs::read_to_string(spine_dir.join("ownership_epoch"))?;
        Ok(content.trim().parse().unwrap_or(1))
    }

    fn set_epoch(&self, repo_id: &str, epoch: u64) -> Result<()> {
        let spine_dir = self.ensure_repo_dirs(repo_id)?;
        fs::write(spine_dir.join("ownership_epoch"), format!("{}\n", epoch))?;
        Ok(())
    }

    fn get_generation(&self, repo_id: &str) -> Result<u64> {
        let spine_dir = self.ensure_repo_dirs(repo_id)?;
        let content = fs::read_to_string(spine_dir.join("generation"))?;
        Ok(content.trim().parse().unwrap_or(0))
    }

    fn set_generation(&self, repo_id: &str, generation: u64) -> Result<()> {
        let spine_dir = self.ensure_repo_dirs(repo_id)?;
        fs::write(spine_dir.join("generation"), format!("{}\n", generation))?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn stage_pack(
        &self,
        tx_id: &str,
        repo_id: &str,
        epoch: u64,
        pack_id: &str,
        pack_checksum: &str,
        pack_bytes: &[u8],
        index_bytes: &[u8],
    ) -> Result<()> {
        let _guard = self.acquire_writer(None)?;
        let current_epoch = self.get_epoch(repo_id)?;
        OwnershipEpoch(current_epoch).validate(epoch)?;

        // Verify checksum of received pack
        let actual_hash = blake3::hash(pack_bytes).to_hex().to_string();
        if actual_hash != pack_checksum {
            bail!(
                "Pack checksum mismatch for {}: expected {}, computed {}",
                pack_id,
                pack_checksum,
                actual_hash
            );
        }

        let stage = self.staging_dir(tx_id);
        fs::create_dir_all(&stage)?;
        fs::write(stage.join("repo_id"), repo_id)?;
        fs::write(stage.join("pack_id"), pack_id)?;
        fs::write(stage.join("epoch"), format!("{}\n", epoch))?;
        fs::write(stage.join(format!("{}.spk", pack_id)), pack_bytes)?;
        fs::write(stage.join(format!("{}.spidx", pack_id)), index_bytes)?;

        Ok(())
    }

    fn commit_pack(
        &self,
        tx_id: &str,
        repo_id: &str,
        epoch: u64,
        new_generation: u64,
    ) -> Result<()> {
        let _guard = self.acquire_writer(Some(new_generation))?;
        let current_epoch = self.get_epoch(repo_id)?;
        OwnershipEpoch(current_epoch).validate(epoch)?;

        let stage = self.staging_dir(tx_id);
        if !stage.exists() {
            bail!("Staged transaction {} not found", tx_id);
        }

        let pack_id = fs::read_to_string(stage.join("pack_id"))?
            .trim()
            .to_string();
        let spine_dir = self.ensure_repo_dirs(repo_id)?;
        let pack_dir = spine_dir.join("pack");

        // Move pack and index into repository
        let src_spk = stage.join(format!("{}.spk", pack_id));
        let dst_spk = pack_dir.join(format!("{}.spk", pack_id));
        let src_spidx = stage.join(format!("{}.spidx", pack_id));
        let dst_spidx = pack_dir.join(format!("{}.spidx", pack_id));

        if src_spk.exists() {
            fs::rename(&src_spk, &dst_spk).or_else(|_| {
                fs::copy(&src_spk, &dst_spk)?;
                fs::remove_file(&src_spk)
            })?;
        }
        if src_spidx.exists() {
            fs::rename(&src_spidx, &dst_spidx).or_else(|_| {
                fs::copy(&src_spidx, &dst_spidx)?;
                fs::remove_file(&src_spidx)
            })?;
        }

        // Clean up staging
        let _ = fs::remove_dir_all(&stage);

        // Update generation
        self.set_generation(repo_id, new_generation)?;
        self.persist_fencing_generation(new_generation)?;
        Ok(())
    }

    fn abort_pack(&self, tx_id: &str) -> Result<()> {
        let _guard = self.acquire_writer(None)?;
        let stage = self.staging_dir(tx_id);
        if stage.exists() {
            fs::remove_dir_all(&stage)?;
        }
        Ok(())
    }

    fn ingest_replicated_pack(
        &self,
        repo_id: &str,
        pack_id: &str,
        pack_checksum: &str,
        pack_bytes: &[u8],
        index_bytes: &[u8],
    ) -> Result<()> {
        let _guard = self.acquire_writer(None)?;
        let actual_hash = blake3::hash(pack_bytes).to_hex().to_string();
        if actual_hash != pack_checksum {
            bail!(
                "Replicated pack checksum mismatch for {}: expected {}, computed {}",
                pack_id,
                pack_checksum,
                actual_hash
            );
        }

        let spine_dir = self.ensure_repo_dirs(repo_id)?;
        let pack_dir = spine_dir.join("pack");
        fs::write(pack_dir.join(format!("{}.spk", pack_id)), pack_bytes)?;
        fs::write(pack_dir.join(format!("{}.spidx", pack_id)), index_bytes)?;

        Ok(())
    }

    fn list_packs(&self, repo_id: &str) -> Result<Vec<String>> {
        let spine_dir = self.ensure_repo_dirs(repo_id)?;
        let pack_dir = spine_dir.join("pack");
        let mut packs = Vec::new();
        if pack_dir.exists() {
            for entry in fs::read_dir(pack_dir)?.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if let Some(id) = name.strip_suffix(".spk") {
                    packs.push(id.to_string());
                }
            }
        }
        packs.sort();
        Ok(packs)
    }

    fn read_pack(&self, repo_id: &str, pack_id: &str) -> Result<(Vec<u8>, Vec<u8>)> {
        let spine_dir = self.ensure_repo_dirs(repo_id)?;
        let pack_dir = spine_dir.join("pack");
        let spk_path = pack_dir.join(format!("{}.spk", pack_id));
        let spidx_path = pack_dir.join(format!("{}.spidx", pack_id));

        if !spk_path.exists() || !spidx_path.exists() {
            bail!("Pack {} not found in repo {}", pack_id, repo_id);
        }

        let spk_bytes = fs::read(&spk_path)?;
        let spidx_bytes = fs::read(&spidx_path)?;
        Ok((spk_bytes, spidx_bytes))
    }

    fn verify_repository(&self, repo_id: &str) -> Result<bool> {
        let packs = self.list_packs(repo_id)?;
        for p in packs {
            let (spk, spidx) = self.read_pack(repo_id, &p)?;
            if spk.len() < 12 || spidx.len() < 1024 {
                return Ok(false);
            }
            if &spk[0..4] != b"SPK1" {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn store_chunk(&self, chunk_hash: &str, bytes: &[u8]) -> Result<()> {
        let _guard = self.acquire_writer(None)?;
        let prefix = if chunk_hash.len() >= 2 {
            &chunk_hash[0..2]
        } else {
            "xx"
        };
        let dir = self.storage_root.join("chunks").join(prefix);
        fs::create_dir_all(&dir)?;
        let path = dir.join(chunk_hash);
        if !path.exists() {
            let tmp = path.with_extension("tmp");
            fs::write(&tmp, bytes)?;
            fs::rename(&tmp, &path)?;
        }
        Ok(())
    }

    fn get_chunk(&self, chunk_hash: &str) -> Result<Option<Vec<u8>>> {
        let prefix = if chunk_hash.len() >= 2 {
            &chunk_hash[0..2]
        } else {
            "xx"
        };
        let path = self
            .storage_root
            .join("chunks")
            .join(prefix)
            .join(chunk_hash);
        if path.exists() {
            Ok(Some(fs::read(&path)?))
        } else {
            Ok(None)
        }
    }

    fn store_manifest(&self, manifest_id: &str, bytes: &[u8]) -> Result<()> {
        let _guard = self.acquire_writer(None)?;
        let prefix = if manifest_id.len() >= 2 {
            &manifest_id[0..2]
        } else {
            "xx"
        };
        let dir = self.storage_root.join("manifests").join(prefix);
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.json", manifest_id));
        if !path.exists() {
            let tmp = path.with_extension("tmp");
            fs::write(&tmp, bytes)?;
            fs::rename(&tmp, &path)?;
        }
        Ok(())
    }

    fn get_manifest(&self, manifest_id: &str) -> Result<Option<Vec<u8>>> {
        let prefix = if manifest_id.len() >= 2 {
            &manifest_id[0..2]
        } else {
            "xx"
        };
        let path = self
            .storage_root
            .join("manifests")
            .join(prefix)
            .join(format!("{}.json", manifest_id));
        if path.exists() {
            Ok(Some(fs::read(&path)?))
        } else {
            Ok(None)
        }
    }

    fn enter_maintenance(&self, fencing_generation: u64) -> Result<()> {
        let _barrier = self
            .mutation_barrier
            .write()
            .map_err(|_| anyhow::anyhow!("Storage mutation barrier poisoned"))?;
        self.in_maintenance.store(true, Ordering::SeqCst);
        self.persist_fencing_generation(fencing_generation)?;

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(5);
        while self.active_writers.load(Ordering::SeqCst) > 0 {
            if start.elapsed() > timeout {
                bail!(
                    "Timed out waiting for {} active storage writers to drain during enter_maintenance",
                    self.active_writers.load(Ordering::SeqCst)
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        Ok(())
    }

    fn exit_maintenance(&self, fencing_generation: u64) -> Result<()> {
        let _barrier = self
            .mutation_barrier
            .write()
            .map_err(|_| anyhow::anyhow!("Storage mutation barrier poisoned"))?;
        self.persist_fencing_generation(fencing_generation)?;
        self.in_maintenance.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn get_maintenance_status(&self) -> (bool, u64, usize) {
        (
            self.in_maintenance.load(Ordering::SeqCst),
            self.fencing_generation.load(Ordering::SeqCst),
            self.active_writers.load(Ordering::SeqCst),
        )
    }

    fn capture_volume_snapshot(
        &self,
        snapshot_id: &str,
        fencing_generation: u64,
    ) -> Result<(VolumeSnapshotManifest, Vec<u8>)> {
        let bounded = spine_cluster::protocol::BoundedVecWriter::new(
            spine_cluster::protocol::MAX_REMOTE_PARTICIPANT_PAYLOAD_BYTES,
        );
        let mut enc = GzEncoder::new(bounded, Compression::default());
        {
            let mut tar = Builder::new(&mut enc);

            let subdirs = ["repos", "chunks", "manifests"];
            for sub in &subdirs {
                let sub_path = self.storage_root.join(sub);
                if sub_path.exists() {
                    tar.append_dir_all(sub, &sub_path)?;
                }
            }
            tar.finish()?;
        }
        let buffer = enc.finish()?.into_inner();

        let sha256_checksum = format!("{:x}", sha2::Sha256::digest(&buffer));
        let total_bytes = buffer.len() as u64;

        let mut file_count = 0usize;
        for sub in &["repos", "chunks", "manifests"] {
            let p = self.storage_root.join(sub);
            if p.exists() {
                file_count += count_files_recursive(&p);
            }
        }

        let now_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let manifest = VolumeSnapshotManifest {
            participant_id: self.node_id.clone(),
            volume_name: format!("{}_data", self.node_id),
            snapshot_id: snapshot_id.to_string(),
            restore_id: None,
            backup_id: Some(snapshot_id.to_string()),
            fencing_generation,
            file_count,
            total_bytes,
            sha256_checksum,
            created_at_nanos: now_nanos,
        };

        Ok((manifest, buffer))
    }

    fn stage_volume_restore(
        &self,
        snapshot_id: &str,
        fencing_generation: u64,
        payload_tar_gz: &[u8],
    ) -> Result<ParticipantAcknowledgement> {
        self.stage_volume_restore_with_manifest(
            snapshot_id,
            fencing_generation,
            None,
            payload_tar_gz,
        )
    }

    fn stage_volume_restore_with_manifest(
        &self,
        snapshot_id: &str,
        fencing_generation: u64,
        manifest: Option<&VolumeSnapshotManifest>,
        payload_tar_gz: &[u8],
    ) -> Result<ParticipantAcknowledgement> {
        if !self.in_maintenance.load(Ordering::SeqCst) {
            bail!("Storage node must be in maintenance mode to stage volume restore");
        }
        let cur_gen = self.fencing_generation.load(Ordering::SeqCst);
        if fencing_generation < cur_gen {
            bail!(
                "Cannot stage volume restore with stale fencing generation {} < {}",
                fencing_generation,
                cur_gen
            );
        }

        let staged_root = self
            .storage_root
            .join(format!(".staged_restore_{}", snapshot_id));
        if staged_root.exists() {
            fs::remove_dir_all(&staged_root)?;
        }
        fs::create_dir_all(&staged_root)?;

        let gz = GzDecoder::new(payload_tar_gz);
        let mut archive = Archive::new(gz);

        for entry in archive.entries()? {
            let mut file = entry?;
            let path = file.path()?.to_path_buf();
            // Reject path traversal: only allow normal relative path components
            for c in path.components() {
                match c {
                    std::path::Component::Normal(_) | std::path::Component::CurDir => {}
                    _ => {
                        bail!(
                            "Unsafe path traversal component detected in storage node snapshot: {:?}",
                            path
                        );
                    }
                }
            }
            let dest = staged_root.join(&path);
            if !dest.starts_with(&staged_root) {
                bail!(
                    "Unsafe destination path outside staging directory: {:?}",
                    dest
                );
            }
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            file.unpack(&dest)?;
        }

        use sha2::Sha256;
        let mut hasher = Sha256::new();
        hasher.update(payload_tar_gz);
        let staged_digest = hex::encode(hasher.finalize());
        let now_nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);

        let (restore_id, backup_id) = match manifest {
            Some(m) => {
                let r_id = m
                    .restore_id
                    .as_deref()
                    .unwrap_or(&format!("rst_{}", snapshot_id))
                    .to_string();
                let b_id = m.backup_id.as_deref().unwrap_or(snapshot_id).to_string();
                (r_id, b_id)
            }
            None => (format!("rst_{}", snapshot_id), snapshot_id.to_string()),
        };

        let staging_rec_rel = format!(".staged_volume_{}/.staged_restore_record.json", snapshot_id);
        let staging_rec_path = self.storage_root.join(&staging_rec_rel);
        if let Some(parent) = staging_rec_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let staging_metadata = serde_json::json!({
            "restore_id": restore_id,
            "backup_id": backup_id,
            "snapshot_id": snapshot_id,
            "node_id": self.node_id,
            "fencing_generation": fencing_generation,
            "staged_digest": staged_digest,
            "staged_bytes": payload_tar_gz.len(),
            "staged_at_nanos": now_nanos,
        });
        if let Ok(m_bytes) = serde_json::to_vec_pretty(&staging_metadata) {
            let tmp_rec = staging_rec_path.with_extension(format!("tmp.{}", uuid::Uuid::new_v4()));
            if let Ok(mut f) = fs::File::create(&tmp_rec) {
                use std::io::Write;
                let _ = f.write_all(&m_bytes);
                let _ = f.flush();
                let _ = f.sync_all();
                let _ = safe_atomic_replace(&tmp_rec, &staging_rec_path);
            }
        }

        let ack = ParticipantAcknowledgement {
            restore_id,
            backup_id,
            participant_id: self.node_id.clone(),
            volume_name: format!("{}_data", self.node_id.replace('-', "_")),
            canonical_resource_id: format!("{}_volume_data", self.node_id.replace('-', "_")),
            previous_generation: Some(cur_gen.to_string()),
            target_generation: fencing_generation.to_string(),
            staged_digest: staged_digest.clone(),
            archive_payload_sha256: staged_digest.clone(),
            staged_resource_digest: staged_digest,
            staged_byte_length: payload_tar_gz.len() as u64,
            prepared_at_nanos: now_nanos,
            prepared: true,
            durable_staging_record_id: staging_rec_rel,
            current_state: "Staged".to_string(),
        };

        Ok(ack)
    }

    fn commit_volume_restore(&self, snapshot_id: &str, fencing_generation: u64) -> Result<()> {
        if !self.in_maintenance.load(Ordering::SeqCst) {
            bail!("Storage node must be in maintenance mode to commit volume restore");
        }
        let cur_gen = self.fencing_generation.load(Ordering::SeqCst);
        if fencing_generation < cur_gen {
            bail!(
                "Cannot commit volume restore with stale fencing generation {} < {}",
                fencing_generation,
                cur_gen
            );
        }

        let staged_root = self
            .storage_root
            .join(format!(".staged_restore_{}", snapshot_id));
        if !staged_root.exists() {
            bail!("Staged restore directory not found for {}", snapshot_id);
        }

        let old_root = self
            .storage_root
            .join(format!(".old_restore_{}", snapshot_id));
        if old_root.exists() {
            let _ = fs::remove_dir_all(&old_root);
        }
        fs::create_dir_all(&old_root)?;

        // Whole-generation replacement: move live subdirs into old_root
        for sub in &["repos", "chunks", "manifests"] {
            let live_sub = self.storage_root.join(sub);
            let old_sub = old_root.join(sub);
            if live_sub.exists() {
                let _ = fs::rename(&live_sub, &old_sub);
            }
            let staged_sub = staged_root.join(sub);
            if staged_sub.exists() {
                fs::rename(&staged_sub, &live_sub)?;
            } else {
                fs::create_dir_all(&live_sub)?;
            }
        }

        // Record storage generation
        let gen_file = self.storage_root.join(".spine_storage_generation");
        fs::write(&gen_file, format!("{}\n", fencing_generation))?;

        // Staged root is now active live data; clean up staging directory
        let _ = fs::remove_dir_all(&staged_root);
        // NOTE: old_root is deliberately RETAINED until global finalize_volume_restore
        // to guarantee rollback is possible if other cluster participants fail.

        self.persist_fencing_generation(fencing_generation)?;
        self.in_maintenance.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn rollback_volume_restore(&self, snapshot_id: &str) -> Result<()> {
        let old_root = self
            .storage_root
            .join(format!(".old_restore_{}", snapshot_id));
        if old_root.exists() {
            for sub in &["repos", "chunks", "manifests"] {
                let live_sub = self.storage_root.join(sub);
                let old_sub = old_root.join(sub);
                if old_sub.exists() {
                    let _ = fs::remove_dir_all(&live_sub);
                    let _ = fs::rename(&old_sub, &live_sub);
                }
            }
            let _ = fs::remove_dir_all(&old_root);
        }
        let staged_root = self
            .storage_root
            .join(format!(".staged_restore_{}", snapshot_id));
        if staged_root.exists() {
            let _ = fs::remove_dir_all(&staged_root);
        }
        Ok(())
    }

    fn finalize_volume_restore(&self, snapshot_id: &str) -> Result<()> {
        let old_root = self
            .storage_root
            .join(format!(".old_restore_{}", snapshot_id));
        if old_root.exists() {
            let _ = fs::remove_dir_all(&old_root);
        }
        let staged_root = self
            .storage_root
            .join(format!(".staged_restore_{}", snapshot_id));
        if staged_root.exists() {
            let _ = fs::remove_dir_all(&staged_root);
        }
        let staging_rec_rel = format!(".staged_volume_{}", snapshot_id);
        let staging_rec_dir = self.storage_root.join(&staging_rec_rel);
        if staging_rec_dir.exists() {
            let _ = fs::remove_dir_all(&staging_rec_dir);
        }
        Ok(())
    }

    fn query_staged_volume_restore(
        &self,
        snapshot_id: &str,
    ) -> Result<Option<ParticipantAcknowledgement>> {
        let staging_rec_rel = format!(".staged_volume_{}/.staged_restore_record.json", snapshot_id);
        let staging_rec_path = self.storage_root.join(&staging_rec_rel);
        if !staging_rec_path.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&staging_rec_path)?;
        let val: serde_json::Value = serde_json::from_slice(&bytes)?;
        let s_id = val
            .get("snapshot_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if s_id != snapshot_id {
            return Ok(None);
        }
        let restore_id = val
            .get("restore_id")
            .and_then(|v| v.as_str())
            .unwrap_or(&format!("rst_{}", snapshot_id))
            .to_string();
        let backup_id = val
            .get("backup_id")
            .and_then(|v| v.as_str())
            .unwrap_or(snapshot_id)
            .to_string();
        let f_gen = val
            .get("fencing_generation")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let staged_digest = val
            .get("staged_digest")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let staged_bytes = val
            .get("staged_bytes")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let now_nanos = val
            .get("staged_at_nanos")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let cur_gen = self.fencing_generation.load(Ordering::SeqCst);

        let ack = ParticipantAcknowledgement {
            restore_id,
            backup_id,
            participant_id: self.node_id.clone(),
            volume_name: format!("{}_data", self.node_id.replace('-', "_")),
            canonical_resource_id: format!("{}_volume_data", self.node_id.replace('-', "_")),
            previous_generation: Some(cur_gen.to_string()),
            target_generation: f_gen.to_string(),
            staged_digest: staged_digest.clone(),
            archive_payload_sha256: staged_digest.clone(),
            staged_resource_digest: staged_digest,
            staged_byte_length: staged_bytes,
            prepared_at_nanos: now_nanos,
            prepared: true,
            durable_staging_record_id: staging_rec_rel,
            current_state: "Staged".to_string(),
        };
        Ok(Some(ack))
    }

    fn verify_volume_integrity(&self) -> Result<(usize, usize, usize)> {
        let mut repos_checked = 0;
        let mut chunks_checked = 0;
        let mut manifests_checked = 0;

        // 1. Verify fencing generation
        let fencing_path = self.storage_root.join(".fencing_generation");
        if !fencing_path.exists() {
            bail!(
                "Integrity check failed: missing .fencing_generation file in {}",
                self.storage_root.display()
            );
        }
        let gen_str = fs::read_to_string(&fencing_path)?;
        let gen = gen_str
            .trim()
            .parse::<u64>()
            .map_err(|e| anyhow::anyhow!("Corrupt fencing generation record: {}", e))?;
        if gen == 0 {
            bail!("Integrity check failed: invalid fencing generation 0");
        }

        // 2. Verify all repositories in repos/
        let repos_dir = self.storage_root.join("repos");
        if repos_dir.exists() {
            for entry in fs::read_dir(&repos_dir)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    let repo_name = entry.file_name().to_string_lossy().to_string();
                    if !self.verify_repository(&repo_name)? {
                        bail!(
                            "Integrity check failed: repository '{}' verification failed",
                            repo_name
                        );
                    }
                    repos_checked += 1;
                }
            }
        }

        // 3. Verify all chunks in chunks/
        let chunks_dir = self.storage_root.join("chunks");
        if chunks_dir.exists() {
            for prefix_entry in fs::read_dir(&chunks_dir)? {
                let prefix_entry = prefix_entry?;
                if prefix_entry.file_type()?.is_dir() {
                    for chunk_entry in fs::read_dir(prefix_entry.path())? {
                        let chunk_entry = chunk_entry?;
                        if chunk_entry.file_type()?.is_file() {
                            let expected_hash =
                                chunk_entry.file_name().to_string_lossy().to_string();
                            if !expected_hash.ends_with(".tmp") {
                                let bytes = fs::read(chunk_entry.path())?;
                                let actual_hash = blake3::hash(&bytes).to_hex().to_string();
                                if actual_hash != expected_hash {
                                    bail!(
                                        "Integrity check failed: chunk hash mismatch for {}. Expected {}, got {}",
                                        chunk_entry.path().display(),
                                        expected_hash,
                                        actual_hash
                                    );
                                }
                                chunks_checked += 1;
                            }
                        }
                    }
                }
            }
        }

        // 4. Verify all manifests in manifests/
        let manifests_dir = self.storage_root.join("manifests");
        if manifests_dir.exists() {
            for prefix_entry in fs::read_dir(&manifests_dir)? {
                let prefix_entry = prefix_entry?;
                if prefix_entry.file_type()?.is_dir() {
                    for manifest_entry in fs::read_dir(prefix_entry.path())? {
                        let manifest_entry = manifest_entry?;
                        if manifest_entry.file_type()?.is_file() {
                            let fname = manifest_entry.file_name().to_string_lossy().to_string();
                            if fname.ends_with(".json") {
                                let expected_id = fname.trim_end_matches(".json");
                                let bytes = fs::read(manifest_entry.path())?;
                                let actual_id = blake3::hash(&bytes).to_hex().to_string();
                                if actual_id != expected_id {
                                    bail!(
                                        "Integrity check failed: manifest hash mismatch for {}. Expected {}, got {}",
                                        manifest_entry.path().display(),
                                        expected_id,
                                        actual_id
                                    );
                                }
                                manifests_checked += 1;
                            }
                        }
                    }
                }
            }
        }

        Ok((repos_checked, chunks_checked, manifests_checked))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_storage_node_writer_guard_and_fencing() {
        let tmp = tempfile::tempdir().unwrap();
        let backend = FileSystemStorageBackend::new(tmp.path(), 100_000_000).unwrap();
        assert_eq!(backend.get_maintenance_status(), (false, 1, 0));

        // Acquire writer without generation requirement
        {
            let _guard = backend
                .acquire_writer(None)
                .expect("Acquire writer without generation");
            assert_eq!(backend.get_maintenance_status(), (false, 1, 1));
        }
        assert_eq!(backend.get_maintenance_status(), (false, 1, 0));

        // Acquire writer with matching/higher generation requirement
        {
            let _guard = backend
                .acquire_writer(Some(2))
                .expect("Acquire writer with gen 2 >= 1");
            assert_eq!(backend.get_maintenance_status(), (false, 1, 1));
        }
        assert_eq!(backend.get_maintenance_status(), (false, 1, 0));

        // Advance fencing generation
        backend.persist_fencing_generation(10).unwrap();
        assert_eq!(backend.get_maintenance_status(), (false, 10, 0));

        // Stale generation must be rejected
        let stale_res = backend.acquire_writer(Some(9));
        assert!(
            stale_res.is_err(),
            "Writer with stale generation 9 < 10 must fail"
        );

        // Enter maintenance
        backend.enter_maintenance(15).unwrap();
        assert_eq!(backend.get_maintenance_status(), (true, 15, 0));

        // In maintenance, any writer must be rejected
        let mut_res = backend.acquire_writer(None);
        assert!(
            mut_res.is_err(),
            "Writer must be rejected during maintenance"
        );

        // Exit maintenance
        backend.exit_maintenance(15).unwrap();
        assert_eq!(backend.get_maintenance_status(), (false, 15, 0));

        // After exiting maintenance, writer succeeds
        let _guard = backend.acquire_writer(Some(15)).unwrap();
        assert_eq!(backend.get_maintenance_status(), (false, 15, 1));
    }

    #[test]
    fn test_capture_volume_snapshot_aborts_early_above_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let backend = FileSystemStorageBackend::new(tmp.path(), 100_000_000).unwrap();
        backend.enter_maintenance(20).unwrap();
        assert_eq!(backend.get_maintenance_status(), (true, 20, 0));

        // Write 18 MiB of incompressible pseudo-random data across chunks/ so gzip output > 15 MiB
        let chunks_dir = tmp.path().join("chunks");
        fs::create_dir_all(&chunks_dir).unwrap();
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut block = vec![0u8; 1024 * 1024];
        for b in &mut block {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *b = (state >> 56) as u8;
        }
        for i in 0..18u8 {
            let mut file_buf = block.clone();
            file_buf[0] ^= i;
            file_buf[1024] ^= i.wrapping_mul(31);
            fs::write(chunks_dir.join(format!("chunk_{:02}.bin", i)), &file_buf).unwrap();
        }

        let err = backend
            .capture_volume_snapshot("snap_oversized", 20)
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("MAX_REMOTE_PARTICIPANT_PAYLOAD_BYTES"),
            "Expected bounded capture abort error, got: {}",
            err
        );

        // Maintenance fencing recovers cleanly after aborted capture
        backend.exit_maintenance(20).unwrap();
        assert_eq!(backend.get_maintenance_status(), (false, 20, 0));
        let _guard = backend.acquire_writer(Some(20)).unwrap();
    }

    #[test]
    fn test_capture_and_restore_production_volume_within_15mib_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let backend = FileSystemStorageBackend::new(tmp.path(), 100_000_000).unwrap();
        backend.enter_maintenance(25).unwrap();

        // Write representative multi-megabyte production dataset (repos, chunks, manifests)
        let repos_dir = tmp.path().join("repos");
        let chunks_dir = tmp.path().join("chunks");
        let manifests_dir = tmp.path().join("manifests");
        fs::create_dir_all(&repos_dir).unwrap();
        fs::create_dir_all(&chunks_dir).unwrap();
        fs::create_dir_all(&manifests_dir).unwrap();

        // 4 MiB of chunks + repos
        let chunk_data = vec![0x42u8; 1024 * 1024]; // 1 MiB per file
        for i in 0..4 {
            fs::write(chunks_dir.join(format!("chunk_{}.pack", i)), &chunk_data).unwrap();
            fs::write(repos_dir.join(format!("repo_{}.idx", i)), &chunk_data).unwrap();
        }
        fs::write(manifests_dir.join("root.manifest"), b"manifest_metadata_v1").unwrap();

        let (manifest, buffer) = backend
            .capture_volume_snapshot("snap_prod_valid", 25)
            .expect("Capture of valid production volume within 15 MiB must succeed");

        assert!(
            buffer.len() <= spine_cluster::protocol::MAX_REMOTE_PARTICIPANT_PAYLOAD_BYTES,
            "Compressed snapshot must be within 15 MiB limit"
        );
        assert_eq!(manifest.total_bytes, buffer.len() as u64);
        assert_eq!(manifest.fencing_generation, 25);
        assert_eq!(manifest.file_count, 9);

        // Verify SHA-256 checksum matches
        let computed_sha256 = format!("{:x}", sha2::Sha256::digest(&buffer));
        assert_eq!(manifest.sha256_checksum, computed_sha256);

        // Stage restore in a separate backend to verify full roundtrip
        let restore_tmp = tempfile::tempdir().unwrap();
        let restore_backend =
            FileSystemStorageBackend::new(restore_tmp.path(), 100_000_000).unwrap();
        restore_backend.enter_maintenance(25).unwrap();

        let ack = restore_backend
            .stage_volume_restore("restore_prod_1", 25, &buffer)
            .expect("Staging restore of valid production volume must succeed");
        assert_eq!(ack.target_generation, "25");
        assert_eq!(ack.archive_payload_sha256, computed_sha256);

        restore_backend
            .commit_volume_restore("restore_prod_1", 25)
            .unwrap();
        restore_backend
            .finalize_volume_restore("restore_prod_1")
            .unwrap();
        restore_backend.exit_maintenance(25).unwrap();

        // Verify restored contents exist and are bit-for-bit identical
        for i in 0..4 {
            let restored_chunk = fs::read(
                restore_tmp
                    .path()
                    .join("chunks")
                    .join(format!("chunk_{}.pack", i)),
            )
            .unwrap();
            assert_eq!(restored_chunk, chunk_data);
        }
    }

    #[test]
    fn test_out_of_band_storage_backup_and_restore_above_15mib() {
        use flate2::read::GzDecoder;
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use tar::{Archive, Builder};

        let tmp = tempfile::tempdir().unwrap();
        let backend = FileSystemStorageBackend::new(tmp.path(), 100_000_000).unwrap();
        backend.enter_maintenance(50).unwrap();
        assert_eq!(backend.get_maintenance_status(), (true, 50, 0));

        // 1. Generate 18 MiB of incompressible data across chunks to exceed 15 MiB limit
        let chunks_dir = tmp.path().join("chunks");
        let repos_dir = tmp.path().join("repos");
        let manifests_dir = tmp.path().join("manifests");
        fs::create_dir_all(&chunks_dir).unwrap();
        fs::create_dir_all(&repos_dir).unwrap();
        fs::create_dir_all(&manifests_dir).unwrap();

        let mut state: u64 = 0xDEAD_BEEF_CAFE_BABE;
        let mut block = vec![0u8; 1024 * 1024]; // 1 MiB block
        for b in &mut block {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            *b = (state >> 56) as u8;
        }

        let mut expected_chunk_hashes = Vec::new();
        for i in 0..18u8 {
            let mut file_buf = block.clone();
            file_buf[0] ^= i;
            file_buf[1024] ^= i.wrapping_mul(43);
            let file_name = format!("chunk_{:02}.bin", i);
            let hash = format!("{:x}", sha2::Sha256::digest(&file_buf));
            expected_chunk_hashes.push((file_name.clone(), hash));
            fs::write(chunks_dir.join(&file_name), &file_buf).unwrap();
        }
        fs::write(
            manifests_dir.join("root.manifest"),
            b"oob_manifest_generation_50",
        )
        .unwrap();

        // 2. Verify in-band single-frame RPC backup aborts immediately to protect memory
        let err = backend
            .capture_volume_snapshot("snap_oversized", 50)
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("MAX_REMOTE_PARTICIPANT_PAYLOAD_BYTES"),
            "In-band capture must abort above 15 MiB: {}",
            err
        );

        // 3. Execute Out-of-Band (OOB) atomic volume backup procedure while quiesced
        let oob_archive_path = tmp.path().join("oob_backup_node_volume.tar.gz");
        {
            let tar_gz_file = fs::File::create(&oob_archive_path).unwrap();
            let encoder = GzEncoder::new(tar_gz_file, Compression::fast());
            let mut tar_builder = Builder::new(encoder);

            // Archive chunks, repos, manifests, and .fencing_generation
            tar_builder.append_dir_all("chunks", &chunks_dir).unwrap();
            tar_builder.append_dir_all("repos", &repos_dir).unwrap();
            tar_builder
                .append_dir_all("manifests", &manifests_dir)
                .unwrap();
            let fence_file = tmp.path().join(".fencing_generation");
            if fence_file.exists() {
                tar_builder
                    .append_path_with_name(&fence_file, ".fencing_generation")
                    .unwrap();
            }
            tar_builder.finish().unwrap();
        }

        let oob_archive_size = fs::metadata(&oob_archive_path).unwrap().len();
        assert!(
            oob_archive_size > spine_cluster::protocol::MAX_REMOTE_PARTICIPANT_PAYLOAD_BYTES as u64,
            "OOB archive must exceed 15 MiB limit to validate large-volume recovery: {} bytes",
            oob_archive_size
        );

        // 4. Perform Out-of-Band Restore onto a fresh storage volume
        let restore_tmp = tempfile::tempdir().unwrap();
        {
            let tar_gz_file = fs::File::open(&oob_archive_path).unwrap();
            let decoder = GzDecoder::new(tar_gz_file);
            let mut archive = Archive::new(decoder);
            archive.unpack(restore_tmp.path()).unwrap();
        }

        // 5. Initialize storage backend on restored volume and verify seamless resumption
        let restored_backend =
            FileSystemStorageBackend::new(restore_tmp.path(), 100_000_000).unwrap();

        // Verify fencing generation restored to 50
        assert_eq!(
            restored_backend.get_maintenance_status().1,
            50,
            "Fencing generation 50 must be preserved by OOB restore"
        );

        // Verify all 18 MiB chunks match expected SHA-256 digests bit-for-bit
        for (file_name, expected_sha256) in &expected_chunk_hashes {
            let restored_data =
                fs::read(restore_tmp.path().join("chunks").join(file_name)).unwrap();
            let actual_sha256 = format!("{:x}", sha2::Sha256::digest(&restored_data));
            assert_eq!(
                &actual_sha256, expected_sha256,
                "Restored chunk {} must match SHA256 exactly",
                file_name
            );
        }

        // Verify manifests
        let restored_manifest =
            fs::read(restore_tmp.path().join("manifests").join("root.manifest")).unwrap();
        assert_eq!(restored_manifest, b"oob_manifest_generation_50");

        // Verify writer permits can be acquired and writes resume cleanly on restored volume
        let _guard = restored_backend.acquire_writer(Some(50)).unwrap();
    }
}
