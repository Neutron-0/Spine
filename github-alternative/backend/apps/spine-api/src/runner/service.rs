use std::sync::Arc;
use chrono::Utc;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use uuid::Uuid;

use crate::core::error::ForgeError;
use crate::core::types::{
    RunnerRegistration, WorkflowJob, WorkflowLogChunk, WorkflowSecret, WorkflowStep,
};
use crate::db::Database;
use crate::workflow::WorkflowScheduler;

pub struct RunnerService {
    db: Arc<dyn Database>,
    scheduler: WorkflowScheduler,
}

impl RunnerService {
    pub fn new(db: Arc<dyn Database>, scheduler: WorkflowScheduler) -> Self {
        Self { db, scheduler }
    }

    /// Register a new build runner and generate a one-time CSPRNG authentication token.
    pub async fn register_runner(
        &self,
        name: &str,
        hostname: &str,
        os: &str,
        arch: &str,
        labels: Vec<String>,
        max_concurrency: i32,
    ) -> Result<(RunnerRegistration, String), ForgeError> {
        let mut raw_token_bytes = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut raw_token_bytes);
        let raw_token = format!("spr_{}", hex::encode(raw_token_bytes));

        let mut hasher = Sha256::new();
        hasher.update(raw_token.as_bytes());
        let token_hash = hex::encode(hasher.finalize());

        let now_nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let registration = RunnerRegistration {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            hostname: hostname.to_string(),
            token_hash,
            os: os.to_string(),
            arch: arch.to_string(),
            labels,
            max_concurrency,
            active_jobs: 0,
            status: "ACTIVE".to_string(),
            last_heartbeat_at_nanos: now_nanos,
            created_at_nanos: now_nanos,
        };

        let saved = self.db.register_runner(registration).await?;
        Ok((saved, raw_token))
    }

    /// Authenticate a runner using its raw registration token.
    pub async fn authenticate_runner(&self, runner_id: &str, raw_token: &str) -> Result<RunnerRegistration, ForgeError> {
        let runner = self
            .db
            .get_runner_by_id(runner_id)
            .await?
            .ok_or_else(|| ForgeError::Unauthorized("Runner not found".to_string()))?;

        if runner.status != "ACTIVE" {
            return Err(ForgeError::Unauthorized("Runner is not active".to_string()));
        }

        let mut hasher = Sha256::new();
        hasher.update(raw_token.as_bytes());
        let computed_hash = hex::encode(hasher.finalize());

        if computed_hash.as_bytes().ct_eq(runner.token_hash.as_bytes()).into() {
            Ok(runner)
        } else {
            Err(ForgeError::Unauthorized("Invalid runner token".to_string()))
        }
    }

    /// Record a runner heartbeat.
    pub async fn heartbeat(&self, runner_id: &str, active_jobs: i32) -> Result<(), ForgeError> {
        let now_nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        self.db.update_runner_heartbeat(runner_id, active_jobs, now_nanos).await
    }

    /// Acquire a queued job matching the runner's supported labels.
    pub async fn acquire_job(
        &self,
        runner_id: &str,
        runner_labels: &[String],
    ) -> Result<Option<(WorkflowJob, Vec<WorkflowStep>, Vec<WorkflowSecret>)>, ForgeError> {
        let now_nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let queued = self.db.list_queued_workflow_jobs(now_nanos).await?;

        for mut job in queued {
            // Check if runner satisfies all runs_on labels
            let can_run = job
                .runs_on
                .iter()
                .all(|req| runner_labels.iter().any(|label| label.eq_ignore_ascii_case(req)));

            if !can_run {
                continue;
            }

            // Assign job with lease and incremented fencing token
            let fencing_token = job.fencing_token + 1;
            // 5 minute initial lease
            let lease_expires_at = now_nanos + (5 * 60 * 1_000_000_000);

            self.db
                .assign_workflow_job(&job.id, runner_id, fencing_token, lease_expires_at)
                .await?;

            job.status = "RUNNING".to_string();
            job.runner_id = Some(runner_id.to_string());
            job.fencing_token = fencing_token;
            job.lease_expires_at_nanos = Some(lease_expires_at);

            let steps = self.db.list_workflow_steps(&job.id).await?;
            let secrets = self.db.list_workflow_secrets("REPO", &job.repo_id).await?;

            return Ok(Some((job, steps, secrets)));
        }

        Ok(None)
    }

    /// Ingest a log chunk with secret masking.
    pub async fn ingest_logs(
        &self,
        runner_id: &str,
        job_id: &str,
        fencing_token: i64,
        step_index: i32,
        stream: &str,
        content: &str,
    ) -> Result<(), ForgeError> {
        let job = self
            .db
            .get_workflow_job(job_id)
            .await?
            .ok_or_else(|| ForgeError::NotFound("Workflow job not found".to_string()))?;

        // Verify lease ownership and fencing token
        if job.runner_id.as_deref() != Some(runner_id) || job.fencing_token != fencing_token {
            return Err(ForgeError::Unauthorized("Stale fencing token or runner lease expired".to_string()));
        }

        // Fetch secrets to mask them in logs
        let secrets = self.db.list_workflow_secrets("REPO", &job.repo_id).await?;
        let mut sanitized = content.to_string();
        for sec in secrets {
            if !sec.ciphertext.is_empty() && sec.ciphertext.len() >= 3 {
                sanitized = sanitized.replace(&sec.ciphertext, "***");
            }
        }

        let chunk = WorkflowLogChunk {
            id: Uuid::new_v4().to_string(),
            job_id: job_id.to_string(),
            step_index,
            stream: stream.to_string(),
            content: sanitized,
            timestamp_nanos: Utc::now().timestamp_nanos_opt().unwrap_or(0),
        };

        self.db.append_workflow_log(chunk).await
    }

    /// Report job completion and advance dependent DAG steps.
    pub async fn complete_job(
        &self,
        runner_id: &str,
        job_id: &str,
        fencing_token: i64,
        exit_code: i32,
        step_results: &[(i32, i32)], // (step_index, exit_code)
    ) -> Result<(), ForgeError> {
        let job = self
            .db
            .get_workflow_job(job_id)
            .await?
            .ok_or_else(|| ForgeError::NotFound("Workflow job not found".to_string()))?;

        if job.runner_id.as_deref() != Some(runner_id) || job.fencing_token != fencing_token {
            return Err(ForgeError::Unauthorized("Stale fencing token or runner lease expired".to_string()));
        }

        let now_nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let final_status = if exit_code == 0 { "SUCCEEDED" } else { "FAILED" };

        // Update steps
        let steps = self.db.list_workflow_steps(job_id).await?;
        for step in steps {
            let res = step_results.iter().find(|(idx, _)| *idx == step.step_index);
            let (code, status) = match res {
                Some((_, 0)) => (Some(0), "SUCCEEDED"),
                Some((_, c)) => (Some(*c), "FAILED"),
                None => (None, "SKIPPED"),
            };
            self.db
                .update_workflow_step(&step.id, status, code, step.started_at_nanos, Some(now_nanos))
                .await?;
        }

        // Update job
        self.db
            .update_workflow_job_status(job_id, final_status, Some(exit_code), job.started_at_nanos, Some(now_nanos))
            .await?;

        // Advance scheduler for repository
        self.scheduler.tick(&job.repo_id).await?;

        Ok(())
    }
}
