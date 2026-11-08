use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use tempfile::tempdir;
use uuid::Uuid;

use spine_api::core::types::{
    BranchProtectionRule, ForgeRepository, PullRequest, TrustClassification, User, WorkflowSecret,
};
use spine_api::db::InMemoryDatabase;
use spine_api::permissions::PermissionService;
use spine_api::runner::{IsolatedRunnerExecutor, RunnerService};
use spine_api::webhook::{SsrfValidator, WebhookService};
use spine_api::workflow::{WorkflowScheduler, WorkflowService};
use spine_api::Database;

#[tokio::test]
async fn test_full_workflow_ci_dag_execution_and_pr_check_protection() {
    let db: Arc<dyn Database> = Arc::new(InMemoryDatabase::new());
    let ssrf = SsrfValidator::new(true);
    let webhooks = WebhookService::new(Arc::clone(&db), ssrf);
    let scheduler = WorkflowScheduler::new(Arc::clone(&db), Some(webhooks.clone()));
    let workflow_service = WorkflowService::new(Arc::clone(&db), Some(webhooks));
    let runner_service = RunnerService::new(Arc::clone(&db), scheduler.clone());
    let permissions = PermissionService::new(Arc::clone(&db));

    let temp_runner_dir = tempdir().unwrap();
    let executor = IsolatedRunnerExecutor::new(temp_runner_dir.path().to_path_buf());

    let now_nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let user_id = Uuid::new_v4().to_string();
    let repo_id = Uuid::new_v4().to_string();
    let commit_oid = "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678".to_string();

    let user = User {
        id: user_id.clone(),
        username: "spine_dev".to_string(),
        normalized_username: "spine_dev".to_string(),
        email: "dev@spine.internal".to_string(),
        normalized_email: "dev@spine.internal".to_string(),
        display_name: "Spine Developer".to_string(),
        password_hash: "argon2id_hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now_nanos,
        updated_at_nanos: now_nanos,
        last_login_nanos: None,
    };
    db.create_user(user.clone()).await.unwrap();

    let repo = ForgeRepository {
        id: repo_id.clone(),
        owner_user_id: user_id.clone(),
        owner_username: "spine_dev".to_string(),
        owner_org_id: None,
        name: "spine-core".to_string(),
        normalized_name: "spine-core".to_string(),
        description: "Core Spine repository".to_string(),
        is_private: false,
        default_branch: "main".to_string(),
        native_repo_id: "repo_native_01".to_string(),
        durability_policy: spine_cluster::durability::DurabilityPolicy::ThreeCopies,
        provisioning_status: spine_api::core::types::ProvisioningStatus::Ready,
        created_at_nanos: now_nanos,
        updated_at_nanos: now_nanos,
        archived: false,
    };
    db.create_repository(repo.clone()).await.unwrap();

    // 1. Configure Branch Protection with required check "CI Pipeline"
    let rule = BranchProtectionRule {
        id: "bpr_main".to_string(),
        repo_id: repo_id.clone(),
        branch_pattern: "main".to_string(),
        require_pull_request: true,
        required_approvals: 0,
        prevent_self_approval: false,
        require_changes_addressed: false,
        dismiss_stale_reviews_on_push: false,
        restrict_merges_to_admins: false,
        required_status_checks: vec!["CI Pipeline".to_string()],
        created_at_nanos: now_nanos,
        updated_at_nanos: now_nanos,
    };
    db.set_branch_protection(rule).await.unwrap();

    // 2. Set repository secret
    let secret = WorkflowSecret {
        id: Uuid::new_v4().to_string(),
        owner_type: "REPO".to_string(),
        owner_id: repo_id.clone(),
        name: "SPINE_API_SECRET".to_string(),
        ciphertext: "super_secret_payload_token_99".to_string(),
        created_at_nanos: now_nanos,
        updated_at_nanos: now_nanos,
    };
    db.upsert_workflow_secret(secret).await.unwrap();

    // 3. Define multi-job DAG workflow with dependencies (.spine/workflows/ci.yml)
    let ci_yaml = r#"
name: "CI Pipeline"
on:
  push:
    branches: [main]
jobs:
  build:
    name: "Build Job"
    runs-on: [windows, self-hosted]
    steps:
      - name: "Build Step"
        run: "echo Compiling Spine..."
  test:
    name: "Test Job"
    needs: [build]
    runs-on: [self-hosted]
    steps:
      - name: "Test Step"
        run: "echo Testing Spine with secret %SPINE_API_SECRET%..."
"#;

    let synced = workflow_service
        .sync_workflows_from_tree(&repo_id, vec![(".spine/workflows/ci.yml", ci_yaml)])
        .await
        .unwrap();
    assert_eq!(synced.len(), 1);
    assert_eq!(synced[0].name, "CI Pipeline");

    // 4. Trigger workflow on push to main
    let runs = workflow_service
        .trigger_workflows(&repo_id, &commit_oid, "refs/heads/main", "push", &user_id)
        .await
        .unwrap();
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    assert_eq!(run.status, "QUEUED");

    // Verify initial PR Check is PENDING
    let checks = db
        .list_workflow_check_results(&repo_id, &commit_oid)
        .await
        .unwrap();
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].check_name, "CI Pipeline");
    assert_eq!(checks[0].status, "PENDING");

    // Verify PR merge is rejected because check is PENDING
    let pr = PullRequest {
        id: "pr_101".to_string(),
        repo_id: repo_id.clone(),
        pr_number: 101,
        title: "Feature branch".to_string(),
        body: "Adding features".to_string(),
        author_id: user_id.clone(),
        base_branch: "main".to_string(),
        head_branch: "feature".to_string(),
        base_commit_id: "0000000000000000000000000000000000000000".to_string(),
        head_commit_id: commit_oid.clone(),
        state: "OPEN".to_string(),
        is_draft: false,
        merged_by_user_id: None,
        merge_commit_id: None,
        merge_strategy: Some("MERGE_COMMIT".to_string()),
        created_at_nanos: now_nanos,
        updated_at_nanos: now_nanos,
        closed_at_nanos: None,
        merged_at_nanos: None,
    };
    db.create_pull_request(pr.clone()).await.unwrap();

    let merge_check = permissions
        .check_merge_allowed(&user, &repo, "main", &pr)
        .await;
    assert!(
        merge_check.is_err(),
        "Merge must be blocked while required check is PENDING"
    );

    // Verify DAG states: build is QUEUED, test is WAITING_FOR_RUNNER
    let jobs = db.list_workflow_jobs(&run.id).await.unwrap();
    let build_job = jobs.iter().find(|j| j.job_key == "build").unwrap();
    let test_job = jobs.iter().find(|j| j.job_key == "test").unwrap();
    assert_eq!(build_job.status, "QUEUED");
    assert_eq!(test_job.status, "WAITING_FOR_RUNNER");

    // 5. Register runner and acquire build job
    let (runner, raw_token) = runner_service
        .register_runner(
            "runner-01",
            "localhost",
            "windows",
            "x86_64",
            vec!["windows".to_string(), "self-hosted".to_string()],
            2,
        )
        .await
        .unwrap();

    let authenticated = runner_service
        .authenticate_runner(&runner.id, &raw_token)
        .await
        .unwrap();
    assert_eq!(authenticated.id, runner.id);

    let acquired = runner_service
        .acquire_job(&runner.id, &runner.labels)
        .await
        .unwrap();
    assert!(acquired.is_some(), "Runner should acquire QUEUED build job");
    let (job1, steps1, secrets1) = acquired.unwrap();
    assert_eq!(job1.job_key, "build");
    assert_eq!(job1.fencing_token, 1);

    // Execute build job steps
    let mut step_scripts = HashMap::new();
    step_scripts.insert(0, "echo Compiling Spine...".to_string());
    let (exit1, results1) = executor
        .execute_job(
            &job1,
            &steps1,
            &step_scripts,
            &secrets1,
            TrustClassification::Trusted,
        )
        .await;
    assert_eq!(exit1, 0);

    // Ingest build job log
    runner_service
        .ingest_logs(
            &runner.id,
            &job1.id,
            job1.fencing_token,
            0,
            "stdout",
            &results1[0].stdout,
        )
        .await
        .unwrap();

    // Complete build job
    runner_service
        .complete_job(&runner.id, &job1.id, job1.fencing_token, 0, &[(0, 0)])
        .await
        .unwrap();

    // 6. Verify DAG: build succeeded, test transitioned from WAITING_FOR_RUNNER to QUEUED
    let updated_test_job = db.get_workflow_job(&test_job.id).await.unwrap().unwrap();
    assert_eq!(
        updated_test_job.status, "QUEUED",
        "Test job must transition to QUEUED once dependencies succeed"
    );

    // 7. Runner acquires and executes test job
    let acquired2 = runner_service
        .acquire_job(&runner.id, &runner.labels)
        .await
        .unwrap();
    assert!(
        acquired2.is_some(),
        "Runner should acquire unblocked test job"
    );
    let (job2, steps2, secrets2) = acquired2.unwrap();
    assert_eq!(job2.job_key, "test");

    let mut step_scripts2 = HashMap::new();
    step_scripts2.insert(
        0,
        "echo Testing Spine with secret %SPINE_API_SECRET%".to_string(),
    );
    let (exit2, results2) = executor
        .execute_job(
            &job2,
            &steps2,
            &step_scripts2,
            &secrets2,
            TrustClassification::Trusted,
        )
        .await;
    assert_eq!(exit2, 0);

    // Verify secret masking: raw secret ciphertext is replaced by ***
    assert!(!results2[0].stdout.contains("super_secret_payload_token_99"));
    assert!(results2[0].stdout.contains("***"));

    // Ingest logs and complete test job
    runner_service
        .ingest_logs(
            &runner.id,
            &job2.id,
            job2.fencing_token,
            0,
            "stdout",
            &results2[0].stdout,
        )
        .await
        .unwrap();

    runner_service
        .complete_job(&runner.id, &job2.id, job2.fencing_token, 0, &[(0, 0)])
        .await
        .unwrap();

    // 8. Verify entire workflow run concluded successfully
    let completed_run = db.get_workflow_run(&run.id).await.unwrap().unwrap();
    assert_eq!(completed_run.status, "COMPLETED");
    assert_eq!(completed_run.conclusion, Some("SUCCESS".to_string()));

    // 9. Verify PR Check Result updated to SUCCESS
    let final_checks = db
        .list_workflow_check_results(&repo_id, &commit_oid)
        .await
        .unwrap();
    assert_eq!(final_checks[0].status, "SUCCESS");

    // 10. Verify Branch Protection now permits PR merge!
    let final_merge_check = permissions
        .check_merge_allowed(&user, &repo, "main", &pr)
        .await;
    assert!(
        final_merge_check.is_ok(),
        "PR merge must succeed when required check 'CI Pipeline' has status 'SUCCESS'"
    );
}
