use anyhow::{bail, Result};
use spine_cluster::fencing::OwnershipEpoch;
use spine_cluster::node::NodeCapacity;
use std::fs;
use std::path::{Path, PathBuf};

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
}

pub struct FileSystemStorageBackend {
    storage_root: PathBuf,
    total_capacity: u64,
}

impl FileSystemStorageBackend {
    pub fn new<P: AsRef<Path>>(storage_root: P, total_capacity: u64) -> Result<Self> {
        let root = storage_root.as_ref().to_path_buf();
        fs::create_dir_all(root.join("repos"))?;
        fs::create_dir_all(root.join("staging"))?;
        fs::create_dir_all(root.join("chunks"))?;
        fs::create_dir_all(root.join("manifests"))?;
        Ok(Self {
            storage_root: root,
            total_capacity,
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
        Ok(())
    }

    fn abort_pack(&self, tx_id: &str) -> Result<()> {
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
}
