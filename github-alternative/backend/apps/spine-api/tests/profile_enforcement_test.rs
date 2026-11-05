use spine_api::{
    core::profile::DeploymentProfile,
    core::types::WorkflowJob,
    db::InMemoryDatabase,
    runner::{IsolatedRunnerExecutor, RunnerService},
    workflow::{WorkflowScheduler, WorkflowService},
};
use std::sync::Arc;

#[test]
fn test_deployment_profile_parsing_and_requirements() {
    assert_eq!(
        DeploymentProfile::from_str_relaxed("core-forge-v1").unwrap(),
        DeploymentProfile::CoreForge
    );
    assert_eq!(
        DeploymentProfile::from_str_relaxed("CORE-FORGE-V1").unwrap(),
        DeploymentProfile::CoreForge
    );
    assert_eq!(
        DeploymentProfile::from_str_relaxed("production").unwrap(),
        DeploymentProfile::CoreForge
    );
    assert_eq!(
        DeploymentProfile::from_str_relaxed("prod").unwrap(),
        DeploymentProfile::CoreForge
    );
    assert_eq!(
        DeploymentProfile::from_str_relaxed("development").unwrap(),
        DeploymentProfile::Development
    );
    assert_eq!(
        DeploymentProfile::from_str_relaxed("test").unwrap(),
        DeploymentProfile::Test
    );

    // Profile requirement properties
    let core_prof = DeploymentProfile::CoreForge;
    assert_eq!(core_prof, DeploymentProfile::CoreForge);
    assert!(core_prof.requires_persistent_database());
    assert!(!core_prof.permits_in_memory_db());
    assert!(!core_prof.allow_untrusted_ci());
    assert!(!core_prof.allow_shared_runners());

    let dev_prof = DeploymentProfile::Development;
    assert_ne!(dev_prof, DeploymentProfile::CoreForge);
    assert!(!dev_prof.requires_persistent_database());
    assert!(dev_prof.permits_in_memory_db());
    assert!(dev_prof.allow_untrusted_ci());
    assert!(dev_prof.allow_shared_runners());
}

#[tokio::test]
async fn test_core_forge_disallows_untrusted_workflow_triggers() {
    let db = Arc::new(InMemoryDatabase::new());
    let workflow_svc =
        WorkflowService::new(db.clone(), None).with_profile(DeploymentProfile::CoreForge);

    let repo_id = "test-repo-uuid-1";

    // Triggering untrusted fork workflow must be disallowed and not queued
    let runs = workflow_svc
        .trigger_workflows(
            repo_id,
            "0000000000000000000000000000000000000001",
            "refs/pull/1/merge",
            "pull_request_fork",
            "attacker_fork_user",
        )
        .await
        .expect("trigger_workflows call succeeded");

    assert!(
        runs.is_empty(),
        "Under CoreForge, untrusted fork trigger must not schedule any runs"
    );
}

#[tokio::test]
async fn test_core_forge_rejects_untrusted_runner_registration() {
    let db = Arc::new(InMemoryDatabase::new());
    let scheduler = WorkflowScheduler::new(db.clone(), None);
    let runner_svc =
        RunnerService::new(db.clone(), scheduler).with_profile(DeploymentProfile::CoreForge);

    // Registering an untrusted/shared runner under core-forge-v1 must be rejected
    let result = runner_svc
        .register_runner(
            "untrusted-external-runner",
            "host-1",
            "linux",
            "x86_64",
            vec!["untrusted".to_string(), "linux".to_string()],
            2,
        )
        .await;

    assert!(
        result.is_err(),
        "Registering a runner with untrusted label under core-forge-v1 must fail"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Untrusted or shared runner registration is disallowed"),
        "Error message must specify core-forge-v1 policy: {}",
        err_msg
    );
}

#[tokio::test]
async fn test_isolated_executor_fails_closed_on_untrusted_execution() {
    let temp_dir = tempfile::tempdir().unwrap();
    let executor = IsolatedRunnerExecutor::new(temp_dir.path().to_path_buf())
        .with_profile(DeploymentProfile::CoreForge);

    let dummy_job = WorkflowJob {
        id: "job-untrusted-1".to_string(),
        run_id: "run-1".to_string(),
        repo_id: "repo-1".to_string(),
        job_key: "build".to_string(),
        name: "test".to_string(),
        status: "QUEUED".to_string(),
        runner_id: None,
        runs_on: vec!["linux".to_string()],
        needs: vec![],
        attempt: 1,
        fencing_token: 1,
        lease_expires_at_nanos: None,
        exit_code: None,
        created_at_nanos: 0,
        started_at_nanos: None,
        completed_at_nanos: None,
    };

    let (exit_code, step_results) = executor
        .execute_job_with_trust(
            &dummy_job,
            &[],
            &std::collections::HashMap::new(),
            &[],
            true, // is_untrusted
        )
        .await;

    assert_eq!(
        exit_code, 1,
        "IsolatedRunnerExecutor must fail closed with exit code 1 for untrusted jobs under core-forge-v1"
    );
    assert!(
        step_results[0]
            .stderr
            .contains("Untrusted workflow execution is strictly disabled"),
        "Stderr must describe untrusted execution prohibition: {}",
        step_results[0].stderr
    );
}
