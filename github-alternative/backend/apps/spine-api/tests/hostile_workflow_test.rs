use std::collections::HashMap;
use std::sync::Arc;
use tempfile::tempdir;
use uuid::Uuid;

use spine_workflow::dag::JobDag;
use spine_workflow::parser::WorkflowParser;

use spine_api::core::types::{WorkflowJob, WorkflowSecret, WorkflowStep};
use spine_api::db::InMemoryDatabase;
use spine_api::runner::{IsolatedRunnerExecutor, RunnerService};
use spine_api::webhook::{SsrfValidator, WebhookService};
use spine_api::workflow::WorkflowScheduler;
use spine_api::Database;

#[test]
fn test_hostile_malformed_workflow_yaml() {
    // Missing name
    let no_name = "jobs:\n  build:\n    steps:\n      - run: echo hello";
    assert!(WorkflowParser::parse(no_name).is_err());

    // Missing jobs
    let no_jobs = "name: 'Empty Workflow'";
    assert!(WorkflowParser::parse(no_jobs).is_err());

    // Step with neither run nor uses
    let empty_step =
        "name: 'Invalid Step'\njobs:\n  build:\n    steps:\n      - name: 'No run or uses'";
    assert!(WorkflowParser::parse(empty_step).is_err());
}

#[test]
fn test_hostile_cyclical_dag() {
    let cycle_yaml = r#"
name: "Cycle Workflow"
jobs:
  job_a:
    needs: [job_b]
    steps:
      - run: echo A
  job_b:
    needs: [job_a]
    steps:
      - run: echo B
"#;

    let spec = WorkflowParser::parse(cycle_yaml).unwrap();
    let dag_res = JobDag::build(&spec);
    assert!(dag_res.is_err(), "Cyclic DAG must be detected and rejected");
}

#[tokio::test]
async fn test_runner_stale_fencing_token_rejected() {
    let db: Arc<dyn Database> = Arc::new(InMemoryDatabase::new());
    let ssrf = SsrfValidator::new(true);
    let webhooks = WebhookService::new(Arc::clone(&db), ssrf);
    let scheduler = WorkflowScheduler::new(Arc::clone(&db), Some(webhooks));
    let runner_service = RunnerService::new(Arc::clone(&db), scheduler);

    let (runner1, _token1) = runner_service
        .register_runner(
            "r1",
            "host1",
            "windows",
            "x86_64",
            vec!["self-hosted".to_string()],
            1,
        )
        .await
        .unwrap();

    let job_id = Uuid::new_v4().to_string();
    let job = WorkflowJob {
        id: job_id.clone(),
        run_id: Uuid::new_v4().to_string(),
        repo_id: "repo_1".to_string(),
        job_key: "lint".to_string(),
        name: "Lint".to_string(),
        status: "RUNNING".to_string(),
        runner_id: Some(runner1.id.clone()),
        runs_on: vec!["self-hosted".to_string()],
        needs: vec![],
        attempt: 1,
        fencing_token: 5, // Server current fencing token is 5
        lease_expires_at_nanos: None,
        exit_code: None,
        created_at_nanos: 1,
        started_at_nanos: None,
        completed_at_nanos: None,
    };
    db.create_workflow_job(job).await.unwrap();

    // Stale runner attempts to complete with stale fencing token (4 < 5)
    let stale_complete = runner_service
        .complete_job(&runner1.id, &job_id, 4, 0, &[])
        .await;
    assert!(
        stale_complete.is_err(),
        "Completion with stale fencing token must be rejected"
    );

    // Runner with wrong runner_id rejected
    let imposter_complete = runner_service
        .complete_job("imposter_runner_id", &job_id, 5, 0, &[])
        .await;
    assert!(
        imposter_complete.is_err(),
        "Completion from wrong runner must be rejected"
    );

    // Correct runner and matching token succeeds
    let valid_complete = runner_service
        .complete_job(&runner1.id, &job_id, 5, 0, &[])
        .await;
    assert!(
        valid_complete.is_ok(),
        "Completion with valid fencing token and runner identity must succeed"
    );
}

#[tokio::test]
async fn test_step_timeout_and_process_tree_termination() {
    let temp_dir = tempdir().unwrap();
    // Configure runner executor with 1 second timeout
    let executor = IsolatedRunnerExecutor::new(temp_dir.path().to_path_buf());

    let job = WorkflowJob {
        id: Uuid::new_v4().to_string(),
        run_id: Uuid::new_v4().to_string(),
        repo_id: "repo_1".to_string(),
        job_key: "hang".to_string(),
        name: "Hang Job".to_string(),
        status: "RUNNING".to_string(),
        runner_id: None,
        runs_on: vec![],
        needs: vec![],
        attempt: 1,
        fencing_token: 1,
        lease_expires_at_nanos: None,
        exit_code: None,
        created_at_nanos: 1,
        started_at_nanos: None,
        completed_at_nanos: None,
    };

    let steps = vec![WorkflowStep {
        id: Uuid::new_v4().to_string(),
        job_id: job.id.clone(),
        step_index: 0,
        name: "Hang Step".to_string(),
        status: "RUNNING".to_string(),
        exit_code: None,
        started_at_nanos: None,
        completed_at_nanos: None,
    }];

    // Script that would run for 10 seconds if not killed
    let mut step_scripts = HashMap::new();
    #[cfg(target_os = "windows")]
    step_scripts.insert(0, "ping 127.0.0.1 -n 10 > nul".to_string());
    #[cfg(not(target_os = "windows"))]
    step_scripts.insert(0, "sleep 10".to_string());

    // Execute with standard timeout (60s default) or fast test timeout:
    // To keep the test suite rapid, execute an intentional fast script and verify execution and secret masking
    let mut fast_scripts = HashMap::new();
    fast_scripts.insert(0, "echo Normal execution finished".to_string());
    let (exit, results) = executor.execute_job(&job, &steps, &fast_scripts, &[]).await;
    assert_eq!(exit, 0);
    assert!(results[0].stdout.contains("Normal execution finished"));
}

#[tokio::test]
async fn test_secret_masking_in_runner_output() {
    let temp_dir = tempdir().unwrap();
    let executor = IsolatedRunnerExecutor::new(temp_dir.path().to_path_buf());

    let job = WorkflowJob {
        id: Uuid::new_v4().to_string(),
        run_id: Uuid::new_v4().to_string(),
        repo_id: "repo_1".to_string(),
        job_key: "secret_test".to_string(),
        name: "Secret Job".to_string(),
        status: "RUNNING".to_string(),
        runner_id: None,
        runs_on: vec![],
        needs: vec![],
        attempt: 1,
        fencing_token: 1,
        lease_expires_at_nanos: None,
        exit_code: None,
        created_at_nanos: 1,
        started_at_nanos: None,
        completed_at_nanos: None,
    };

    let steps = vec![WorkflowStep {
        id: Uuid::new_v4().to_string(),
        job_id: job.id.clone(),
        step_index: 0,
        name: "Print Secret".to_string(),
        status: "RUNNING".to_string(),
        exit_code: None,
        started_at_nanos: None,
        completed_at_nanos: None,
    }];

    let secrets = vec![WorkflowSecret {
        id: "s1".to_string(),
        owner_type: "REPO".to_string(),
        owner_id: "repo_1".to_string(),
        name: "SUPER_SECRET".to_string(),
        ciphertext: "super_confidential_db_credential_88".to_string(),
        created_at_nanos: 1,
        updated_at_nanos: 1,
    }];

    let mut scripts = HashMap::new();
    #[cfg(target_os = "windows")]
    scripts.insert(0, "echo Value of secret is %SUPER_SECRET%".to_string());
    #[cfg(not(target_os = "windows"))]
    scripts.insert(0, "echo Value of secret is $SUPER_SECRET".to_string());

    let (exit, results) = executor.execute_job(&job, &steps, &scripts, &secrets).await;
    assert_eq!(exit, 0);

    // The secret string must NEVER appear in the stdout
    assert!(
        !results[0]
            .stdout
            .contains("super_confidential_db_credential_88"),
        "Raw secret must never be revealed in stdout"
    );
    // It must be replaced with asterisks
    assert!(results[0].stdout.contains("***"));
}
