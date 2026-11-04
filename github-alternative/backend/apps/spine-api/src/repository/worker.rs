use crate::coordinator::CoordinatorClient;
use crate::core::types::*;
use crate::db::Database;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::time::{sleep, Duration};

pub struct ProvisioningWorker {
    db: Arc<dyn Database>,
    coordinator: Option<CoordinatorClient>,
    storage_root: PathBuf,
    running: Arc<AtomicBool>,
}

impl ProvisioningWorker {
    pub fn new<P: AsRef<Path>>(
        db: Arc<dyn Database>,
        coordinator: Option<CoordinatorClient>,
        storage_root: P,
    ) -> Self {
        Self {
            db,
            coordinator,
            storage_root: storage_root.as_ref().to_path_buf(),
            running: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn shutdown(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Process a single pending provisioning job (useful for deterministic tests).
    pub async fn process_next_job(&self) -> anyhow::Result<bool> {
        let job = match self.db.get_next_pending_job().await? {
            Some(j) => j,
            None => return Ok(false),
        };

        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let repo = match self.db.get_repository_by_id(&job.repo_id).await? {
            Some(r) => r,
            None => {
                let _ = self
                    .db
                    .update_job_status(
                        &job.id,
                        ProvisioningStatus::Failed,
                        Some("Repository record missing".into()),
                        now,
                    )
                    .await;
                return Ok(true);
            }
        };

        let _ = self
            .db
            .update_job_status(&job.id, ProvisioningStatus::Provisioning, None, now)
            .await;
        let _ = self
            .db
            .update_repository_status(&repo.id, ProvisioningStatus::Provisioning, now)
            .await;

        // 1. If coordinator configured, request placement
        if let Some(ref coord) = self.coordinator {
            if let Err(e) = coord
                .allocate_repository(
                    &repo.native_repo_id,
                    &repo.name,
                    &repo.owner_username,
                    repo.durability_policy,
                )
                .await
            {
                let err_msg = format!("Coordinator allocation error: {}", e);
                let next_status = if job.attempts >= 4 {
                    ProvisioningStatus::Failed
                } else {
                    ProvisioningStatus::Pending
                };
                let _ = self
                    .db
                    .update_job_status(&job.id, next_status, Some(err_msg.clone()), now)
                    .await;
                let _ = self
                    .db
                    .update_repository_status(&repo.id, next_status, now)
                    .await;
                return Err(anyhow::anyhow!(err_msg));
            }
        }

        // 2. Initialize native Spine repository structure on filesystem
        let repo_dir = self.storage_root.join("repos").join(&repo.native_repo_id);
        let spine_dir = repo_dir.join(".spine");
        fs::create_dir_all(spine_dir.join("objects").join("pack"))?;
        fs::create_dir_all(spine_dir.join("refs").join("heads"))?;
        fs::create_dir_all(spine_dir.join("refs").join("tags"))?;

        let epoch_file = spine_dir.join("ownership_epoch");
        if !epoch_file.exists() {
            fs::write(&epoch_file, "1\n")?;
        }
        let gen_file = spine_dir.join("generation");
        if !gen_file.exists() {
            fs::write(&gen_file, "0\n")?;
        }
        let head_file = spine_dir.join("HEAD");
        if !head_file.exists() {
            fs::write(&head_file, "ref: refs/heads/main\n")?;
        }
        let config_file = spine_dir.join("config");
        if !config_file.exists() {
            fs::write(&config_file, "[core]\n\tformatversion = 2\n")?;
        }

        // 3. Mark job and repository as READY
        let completion_time = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        self.db
            .update_job_status(&job.id, ProvisioningStatus::Ready, None, completion_time)
            .await?;
        self.db
            .update_repository_status(&repo.id, ProvisioningStatus::Ready, completion_time)
            .await?;

        Ok(true)
    }

    /// Background worker loop.
    pub async fn run(&self) {
        while self.running.load(Ordering::SeqCst) {
            match self.process_next_job().await {
                Ok(true) => {
                    // Processed a job, immediately check for more
                    continue;
                }
                Ok(false) => {
                    // No jobs pending, sleep briefly
                    sleep(Duration::from_millis(500)).await;
                }
                Err(e) => {
                    tracing::warn!("Provisioning worker error: {}", e);
                    sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }
}
