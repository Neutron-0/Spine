use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::timeout;

use crate::core::types::{WorkflowJob, WorkflowSecret, WorkflowStep};

#[derive(Debug, Clone)]
pub struct StepExecutionResult {
    pub step_index: i32,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

pub struct IsolatedRunnerExecutor {
    workspace_root: PathBuf,
    default_timeout: Duration,
}

impl IsolatedRunnerExecutor {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self {
            workspace_root,
            default_timeout: Duration::from_secs(60),
        }
    }

    /// Execute all steps in an isolated workspace with sanitized environment and timeout enforcement.
    pub async fn execute_job(
        &self,
        job: &WorkflowJob,
        steps: &[WorkflowStep],
        step_scripts: &HashMap<i32, String>,
        secrets: &[WorkflowSecret],
    ) -> (i32, Vec<StepExecutionResult>) {
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
