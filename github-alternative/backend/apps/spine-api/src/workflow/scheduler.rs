use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;

use crate::core::error::ForgeError;
use crate::core::types::WorkflowCheckResult;
use crate::db::Database;
use crate::webhook::WebhookService;

#[derive(Clone)]
pub struct WorkflowScheduler {
    db: Arc<dyn Database>,
    webhook_service: Option<WebhookService>,
}

impl WorkflowScheduler {
    pub fn new(db: Arc<dyn Database>, webhook_service: Option<WebhookService>) -> Self {
        Self {
            db,
            webhook_service,
        }
    }

    /// Advance active workflow runs, resolve job dependencies, detect timed out leases,
    /// and finalize completed runs.
    pub async fn tick(&self, repo_id: &str) -> Result<(), ForgeError> {
        let now_nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let runs = self.db.list_workflow_runs(repo_id, 100).await?;

        for run in runs {
            if run.status == "COMPLETED" || run.status == "CANCELLED" {
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
                        .map_or(false, |exp| exp < now_nanos)
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
