use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::timeout;

use crate::core::error::ForgeError;
use crate::core::types::{TrustClassification, WorkflowJob, WorkflowSecret, WorkflowStep};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerSandboxMode {
    /// Explicitly marked for trusted local development runs only.
    TrustedLocalDevelopment,
    /// OCI-compliant container isolation (Linux).
    SandboxedOci,
    /// Missing or unverified sandbox boundary on the host platform.
    Unsandboxed,
}

#[derive(Debug, Clone)]
pub struct StepExecutionResult {
    pub step_index: i32,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

use crate::core::profile::DeploymentProfile;

/// Authoritative provenance trust token validating execution authority.
/// Callers cannot manufacture trust arbitrarily; it must be resolved authoritatively from the database record.
#[derive(Debug, Clone)]
pub struct AuthoritativeTrustToken {
    pub(crate) run_id: String,
    pub(crate) job_id: String,
    pub(crate) repository_id: String,
    pub(crate) classification: TrustClassification,
    pub(crate) is_fork: bool,
    pub(crate) source_repo_id: Option<String>,
}

impl AuthoritativeTrustToken {
    /// Authoritatively load workflow run trust token directly from database.
    /// Callers cannot spoof or assert trust; it is validated against the recorded workflow_job and workflow_run.
    pub async fn from_database(
        db: &dyn crate::db::Database,
        job: &WorkflowJob,
    ) -> Result<Self, ForgeError> {
        let persisted_job = db.get_workflow_job(&job.id).await?.ok_or_else(|| {
            ForgeError::NotFound(format!("Workflow job '{}' not found in database", job.id))
        })?;

        if persisted_job.id != job.id {
            return Err(ForgeError::Unauthorized(
                "Job ID mismatch with persisted database record".to_string(),
            ));
        }
        if persisted_job.run_id != job.run_id {
            return Err(ForgeError::Unauthorized(
                "Job run_id mismatch with persisted database record".to_string(),
            ));
        }
        if persisted_job.repo_id.trim().is_empty() {
            return Err(ForgeError::Unauthorized(
                "Persisted workflow job has empty repository ID".to_string(),
            ));
        }
        if persisted_job.repo_id != job.repo_id {
            return Err(ForgeError::Unauthorized(
                "Job repo_id mismatch with persisted database record".to_string(),
            ));
        }

        let run = db.get_workflow_run(&job.run_id).await?.ok_or_else(|| {
            ForgeError::NotFound(format!(
                "Workflow run '{}' not found in database",
                job.run_id
            ))
        })?;

        if run.id != job.run_id || run.id != persisted_job.run_id {
            return Err(ForgeError::Unauthorized(
                "Job run_id mismatch with workflow run".to_string(),
            ));
        }
        if run.repo_id.trim().is_empty() {
            return Err(ForgeError::Unauthorized(
                "Persisted workflow run has empty repository ID".to_string(),
            ));
        }
        if run.repo_id != job.repo_id || run.repo_id != persisted_job.repo_id {
            return Err(ForgeError::Unauthorized(
                "Job repo_id mismatch with workflow run".to_string(),
            ));
        }
        if run.is_fork && run.trust_classification != TrustClassification::Untrusted {
            return Err(ForgeError::Unauthorized(
                "Fork repository runs must be classified as untrusted".to_string(),
            ));
        }

        Ok(Self {
            run_id: run.id,
            job_id: persisted_job.id,
            repository_id: run.repo_id,
            classification: run.trust_classification,
            is_fork: run.is_fork,
            source_repo_id: run.source_repo_id,
        })
    }

    /// Factory method for test harnesses ONLY.
    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test(
        run_id: &str,
        job_id: &str,
        repository_id: &str,
        classification: TrustClassification,
    ) -> Self {
        Self {
            run_id: run_id.to_string(),
            job_id: job_id.to_string(),
            repository_id: repository_id.to_string(),
            classification,
            is_fork: classification == TrustClassification::Untrusted,
            source_repo_id: None,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn for_test_full(
        run_id: &str,
        job_id: &str,
        repository_id: &str,
        classification: TrustClassification,
        is_fork: bool,
        source_repo_id: Option<String>,
    ) -> Self {
        Self {
            run_id: run_id.to_string(),
            job_id: job_id.to_string(),
            repository_id: repository_id.to_string(),
            classification,
            is_fork,
            source_repo_id,
        }
    }

    pub fn classification(&self) -> TrustClassification {
        self.classification
    }

    pub fn is_untrusted(&self) -> bool {
        self.classification == TrustClassification::Untrusted
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn job_id(&self) -> &str {
        &self.job_id
    }

    pub fn repository_id(&self) -> &str {
        &self.repository_id
    }

    pub fn is_fork(&self) -> bool {
        self.is_fork
    }

    pub fn source_repo_id(&self) -> Option<&str> {
        self.source_repo_id.as_deref()
    }
}

pub struct IsolatedRunnerExecutor {
    workspace_root: PathBuf,
    default_timeout: Duration,
    sandbox_mode: RunnerSandboxMode,
    pub profile: DeploymentProfile,
}

impl IsolatedRunnerExecutor {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self {
            workspace_root,
            default_timeout: Duration::from_secs(60),
            sandbox_mode: RunnerSandboxMode::TrustedLocalDevelopment,
            profile: DeploymentProfile::CoreForge,
        }
    }

    pub fn with_profile(mut self, profile: DeploymentProfile) -> Self {
        self.profile = profile;
        self
    }

    pub fn with_sandbox_mode(mut self, mode: RunnerSandboxMode) -> Self {
        self.sandbox_mode = mode;
        self
    }

    pub fn sandbox_mode(&self) -> RunnerSandboxMode {
        self.sandbox_mode
    }

    /// Execute all steps in an isolated workspace with sanitized environment and authoritative provenance verification.
    pub async fn execute_job(
        &self,
        job: &WorkflowJob,
        steps: &[WorkflowStep],
        step_scripts: &HashMap<i32, String>,
        secrets: &[WorkflowSecret],
        trust_token: &AuthoritativeTrustToken,
    ) -> (i32, Vec<StepExecutionResult>) {
        if trust_token.job_id() != job.id
            || trust_token.run_id() != job.run_id
            || trust_token.repository_id().trim().is_empty()
            || trust_token.repository_id() != job.repo_id
        {
            return (
                1,
                vec![StepExecutionResult {
                    step_index: 0,
                    exit_code: 1,
                    stdout: String::new(),
                    stderr:
                        "SECURITY ERROR: Trust token does not match job/run/repository identity or repository ID is empty"
                            .to_string(),
                    timed_out: false,
                }],
            );
        }

        let is_untrusted = trust_token.is_untrusted();

        if is_untrusted && !self.profile.allow_untrusted_ci() {
            return (
                1,
                vec![StepExecutionResult {
                    step_index: 0,
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: "SECURITY ERROR: Untrusted workflow execution is strictly disabled on this deployment profile ('core-forge-v1').".to_string(),
                    timed_out: false,
                }],
            );
        }

        if is_untrusted && self.sandbox_mode == RunnerSandboxMode::Unsandboxed {
            return (
                1,
                vec![StepExecutionResult {
                    step_index: 0,
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: "SECURITY ERROR: Untrusted workflow execution requires a verified sandbox (e.g. OCI container isolation). Unsandboxed execution of untrusted workflows is disabled on this platform/backend to prevent host compromise. Configure an isolated runner or explicitly mark execution as trusted local development mode.".to_string(),
                    timed_out: false,
                }],
            );
        }
        let job_workspace = self.workspace_root.join(&job.id);
        if let Err(e) = tokio::fs::create_dir_all(&job_workspace).await {
            tracing::error!("Failed to create job workspace {:?}: {}", job_workspace, e);
            return (
                1,
                vec![StepExecutionResult {
                    step_index: 0,
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: format!("Workspace initialization failed: {}", e),
                    timed_out: false,
                }],
            );
        }

        let mut results = Vec::new();
        let mut overall_exit_code = 0;

        for step in steps {
            let script_content = step_scripts
                .get(&step.step_index)
                .cloned()
                .unwrap_or_else(|| "echo No command specified".to_string());

            let res = self
                .execute_single_step(job, step, &script_content, secrets, &job_workspace)
                .await;

            let failed = res.exit_code != 0 || res.timed_out;
            results.push(res);

            if failed {
                overall_exit_code = 1;
                break;
            }
        }

        // Clean up workspace
        let _ = tokio::fs::remove_dir_all(&job_workspace).await;

        (overall_exit_code, results)
    }

    async fn execute_single_step(
        &self,
        job: &WorkflowJob,
        step: &WorkflowStep,
        script: &str,
        secrets: &[WorkflowSecret],
        workspace: &Path,
    ) -> StepExecutionResult {
        #[cfg(target_os = "windows")]
        let script_file = workspace.join(format!("step_{}.bat", step.step_index));
        #[cfg(not(target_os = "windows"))]
        let script_file = workspace.join(format!("step_{}.sh", step.step_index));

        #[cfg(target_os = "windows")]
        let formatted_script = format!("@echo off\r\nchcp 65001 > nul\r\n{}", script);
        #[cfg(not(target_os = "windows"))]
        let formatted_script = format!("#!/bin/sh\nset -e\n{}", script);

        if let Err(e) = tokio::fs::write(&script_file, &formatted_script).await {
            return StepExecutionResult {
                step_index: step.step_index,
                exit_code: 1,
                stdout: String::new(),
                stderr: format!("Failed to write step script: {}", e),
                timed_out: false,
            };
        }

        #[cfg(target_os = "windows")]
        let mut cmd = Command::new("cmd.exe");
        #[cfg(target_os = "windows")]
        cmd.args(["/c", script_file.to_str().unwrap()]);

        #[cfg(not(target_os = "windows"))]
        let mut cmd = Command::new("sh");
        #[cfg(not(target_os = "windows"))]
        cmd.arg(&script_file);

        // Sanitize environment: clean slate + essential PATH/system vars
        cmd.env_clear();
        for (k, v) in std::env::vars() {
            let k_upper = k.to_uppercase();
            if matches!(
                k_upper.as_str(),
                "PATH"
                    | "SYSTEMROOT"
                    | "COMSPEC"
                    | "PATHEXT"
                    | "TEMP"
                    | "TMP"
                    | "USERPROFILE"
                    | "HOME"
                    | "NUMBER_OF_PROCESSORS"
                    | "OS"
            ) {
                cmd.env(k, v);
            }
        }

        // Inject standard workflow environment variables
        cmd.env("SPINE_RUN_ID", &job.run_id);
        cmd.env("SPINE_JOB_ID", &job.id);
        cmd.env("SPINE_JOB_KEY", &job.job_key);
        cmd.env("SPINE_WORKSPACE", workspace.to_string_lossy().as_ref());

        // Inject secrets
        for sec in secrets {
            cmd.env(&sec.name, &sec.ciphertext);
        }

        cmd.current_dir(workspace);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return StepExecutionResult {
                    step_index: step.step_index,
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: format!("Failed to spawn step process: {}", e),
                    timed_out: false,
                };
            }
        };

        let mut stdout_pipe = child.stdout.take();
        let mut stderr_pipe = child.stderr.take();

        let wait_fut = child.wait();

        match timeout(self.default_timeout, wait_fut).await {
            Ok(Ok(status)) => {
                let mut stdout_buf = Vec::new();
                if let Some(ref mut p) = stdout_pipe {
                    let _ = p.read_to_end(&mut stdout_buf).await;
                }

                let mut stderr_buf = Vec::new();
                if let Some(ref mut p) = stderr_pipe {
                    let _ = p.read_to_end(&mut stderr_buf).await;
                }

                let raw_stdout = String::from_utf8_lossy(&stdout_buf).to_string();
                let raw_stderr = String::from_utf8_lossy(&stderr_buf).to_string();

                let stdout = mask_secrets(&raw_stdout, secrets);
                let stderr = mask_secrets(&raw_stderr, secrets);

                let exit_code = status.code().unwrap_or(1);
                StepExecutionResult {
                    step_index: step.step_index,
                    exit_code,
                    stdout,
                    stderr,
                    timed_out: false,
                }
            }
            Ok(Err(e)) => StepExecutionResult {
                step_index: step.step_index,
                exit_code: 1,
                stdout: String::new(),
                stderr: format!("Process execution error: {}", e),
                timed_out: false,
            },
            Err(_) => {
                // Process timed out: kill process tree
                #[cfg(target_os = "windows")]
                if let Some(pid) = child.id() {
                    let _ = std::process::Command::new("taskkill")
                        .args(["/PID", &pid.to_string(), "/T", "/F"])
                        .output();
                }
                let _ = child.kill().await;

                StepExecutionResult {
                    step_index: step.step_index,
                    exit_code: 124,
                    stdout: String::new(),
                    stderr: format!("Step timed out after {:?}", self.default_timeout),
                    timed_out: true,
                }
            }
        }
    }
}

fn mask_secrets(input: &str, secrets: &[WorkflowSecret]) -> String {
    let mut masked = input.to_string();
    for sec in secrets {
        if !sec.ciphertext.is_empty() && sec.ciphertext.len() >= 3 {
            masked = masked.replace(&sec.ciphertext, "***");
        }
    }
    masked
}
