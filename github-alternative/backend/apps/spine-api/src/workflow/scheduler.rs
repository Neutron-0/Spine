use chrono::Utc;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::core::error::ForgeError;
use crate::core::types::WorkflowCheckResult;
use crate::db::Database;
use crate::webhook::WebhookService;

use crate::core::profile::DeploymentProfile;

#[derive(Clone)]
pub struct WorkflowScheduler {
    db: Arc<dyn Database>,
    webhook_service: Option<WebhookService>,
    pub profile: DeploymentProfile,
    consistency_barrier: Option<Arc<crate::backup::ConsistencyBarrier>>,
    running: Arc<AtomicBool>,
}

impl WorkflowScheduler {
    pub fn new(db: Arc<dyn Database>, webhook_service: Option<WebhookService>) -> Self {
        Self {
            db,
            webhook_service,
            profile: DeploymentProfile::CoreForge,
            consistency_barrier: None,
            running: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn with_profile(mut self, profile: DeploymentProfile) -> Self {
        self.profile = profile;
        self
    }

    pub fn with_barrier(mut self, barrier: Arc<crate::backup::ConsistencyBarrier>) -> Self {
        self.consistency_barrier = Some(barrier);
        self
    }

    pub fn shutdown(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Advance active workflow runs across all repositories with pending/running workflows.
    pub async fn tick_active(&self) -> Result<(), ForgeError> {
        let repo_ids = self.db.list_active_workflow_repo_ids().await?;
        for repo_id in repo_ids {
            if let Err(e) = self.tick(&repo_id).await {
                tracing::warn!("Scheduler tick failed for repo {}: {}", repo_id, e);
            }
        }
        Ok(())
    }

    /// Background scheduler loop.
    pub async fn run(&self) {
        while self.running.load(Ordering::SeqCst) {
            if let Err(e) = self.tick_active().await {
                tracing::warn!("WorkflowScheduler tick error: {}", e);
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }

    /// Advance active workflow runs, resolve job dependencies, detect timed out leases,
    /// and finalize completed runs.
    pub async fn tick(&self, repo_id: &str) -> Result<(), ForgeError> {
        let _permit = if let Some(barrier) = &self.consistency_barrier {
            match barrier.acquire_write_permit().await {
                Ok(p) => Some(p),
                Err(e) => {
                    tracing::warn!("WorkflowScheduler paused during cluster maintenance: {}", e);
                    return Ok(());
                }
            }
        } else {
            None
        };

        let now_nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let runs = self.db.list_active_workflow_runs_for_repo(repo_id).await?;

        for run in runs {
            if run.status == "COMPLETED" || run.status == "CANCELLED" {
                continue;
            }

            let is_untrusted_run =
                run.trust_classification == crate::core::types::TrustClassification::Untrusted;

            if is_untrusted_run && !self.profile.allow_untrusted_ci() {
                tracing::warn!(
                    "Scheduler closing untrusted workflow run {} under profile '{}'",
                    run.id,
                    self.profile.name()
                );
                let _ = self
                    .db
                    .update_workflow_run_status(
                        &run.id,
                        "COMPLETED",
                        Some("DISALLOWED"),
                        None,
                        Some(now_nanos),
                    )
                    .await;
                continue;
            }

            let jobs = self.db.list_workflow_jobs(&run.id).await?;
            if jobs.is_empty() {
                continue;
            }

            // Build map of job_key -> status
            let mut job_status_map: HashMap<String, String> = HashMap::new();
            for job in &jobs {
                job_status_map.insert(job.job_key.clone(), job.status.clone());
            }

            // Check for timed out leases on ASSIGNED / RUNNING jobs
            for job in &jobs {
                if (job.status == "ASSIGNED" || job.status == "RUNNING")
                    && job
                        .lease_expires_at_nanos
                        .is_some_and(|exp| exp < now_nanos)
                {
                    tracing::warn!("Job {} lease expired; marking TIMED_OUT", job.id);
                    self.db
                        .update_workflow_job_status(
                            &job.id,
                            "TIMED_OUT",
                            Some(124),
                            None,
                            Some(now_nanos),
                        )
                        .await?;
                    job_status_map.insert(job.job_key.clone(), "TIMED_OUT".to_string());
                }
            }

            // Advance downstream jobs waiting on dependencies
            for job in &jobs {
                if job.status == "WAITING_FOR_RUNNER" && !job.needs.is_empty() {
                    let mut all_succeeded = true;
                    let mut any_failed = false;

                    for dep in &job.needs {
                        match job_status_map.get(dep).map(|s| s.as_str()) {
                            Some("SUCCEEDED") => {}
                            Some("FAILED") | Some("TIMED_OUT") | Some("CANCELLED") => {
                                all_succeeded = false;
                                any_failed = true;
                                break;
                            }
                            _ => {
                                all_succeeded = false;
                            }
                        }
                    }

                    if all_succeeded {
                        // All dependencies passed; queue this job for runner pickup
                        self.db
                            .update_workflow_job_status(&job.id, "QUEUED", None, None, None)
                            .await?;
                        job_status_map.insert(job.job_key.clone(), "QUEUED".to_string());
                    } else if any_failed {
                        // Dependency failed; skip this job
                        self.db
                            .update_workflow_job_status(
                                &job.id,
                                "SKIPPED",
                                Some(0),
                                None,
                                Some(now_nanos),
                            )
                            .await?;
                        job_status_map.insert(job.job_key.clone(), "SKIPPED".to_string());
                    }
                }
            }

            // Check if all jobs have reached a terminal state
            let all_terminal = jobs.iter().all(|j| {
                let status = job_status_map
                    .get(&j.job_key)
                    .map(|s| s.as_str())
                    .unwrap_or(&j.status);
                matches!(
                    status,
                    "SUCCEEDED" | "FAILED" | "TIMED_OUT" | "CANCELLED" | "SKIPPED"
                )
            });

            if all_terminal {
                let mut conclusion = "SUCCESS";
                for j in &jobs {
                    let status = job_status_map
                        .get(&j.job_key)
                        .map(|s| s.as_str())
                        .unwrap_or(&j.status);
                    if status == "FAILED" || status == "TIMED_OUT" {
                        conclusion = "FAILURE";
                        break;
                    } else if status == "CANCELLED" {
                        conclusion = "CANCELLED";
                    }
                }

                // Finalize run
                self.db
                    .update_workflow_run_status(
                        &run.id,
                        "COMPLETED",
                        Some(conclusion),
                        None,
                        Some(now_nanos),
                    )
                    .await?;

                // Update PR Check Result
                let check_result = WorkflowCheckResult {
                    id: uuid::Uuid::new_v4().to_string(),
                    repo_id: run.repo_id.clone(),
                    commit_oid: run.commit_oid.clone(),
                    check_name: run.workflow_name.clone(),
                    status: conclusion.to_string(),
                    run_id: run.id.clone(),
                    updated_at_nanos: now_nanos,
                };
                let _ = self.db.upsert_workflow_check_result(check_result).await;

                // Dispatch webhook for workflow_run.completed
                if let Some(wh) = &self.webhook_service {
                    let payload = serde_json::json!({
                        "action": "completed",
                        "workflow_run_id": run.id,
                        "conclusion": conclusion,
                        "repository_id": run.repo_id,
                        "commit_oid": run.commit_oid,
                    });
                    let _ = wh
                        .dispatch_event("REPO", &run.repo_id, "workflow_run", &payload)
                        .await;
                }
            }
        }

        Ok(())
    }
}
