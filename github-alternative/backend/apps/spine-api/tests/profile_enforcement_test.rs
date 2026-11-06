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

#[tokio::test]
async fn test_provenance_evaluation_matrix() {
    use spine_api::core::types::*;
    use spine_api::db::Database;
    use spine_cluster::durability::DurabilityPolicy;

    let db = Arc::new(InMemoryDatabase::new());
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

    // Create target repo owned by user-owner
    let target_repo = ForgeRepository {
        id: "target-repo-1".to_string(),
        owner_user_id: "user-owner".to_string(),
        owner_username: "owner".to_string(),
        owner_org_id: None,
        name: "target-repo".to_string(),
        normalized_name: "target-repo".to_string(),
        description: "Test repo".to_string(),
        is_private: false,
        default_branch: "main".to_string(),
        native_repo_id: "target-repo-1".to_string(),
        durability_policy: DurabilityPolicy::ThreeCopies,
        provisioning_status: ProvisioningStatus::Ready,
        created_at_nanos: now,
        updated_at_nanos: now,
        archived: false,
    };
    db.create_repository(target_repo).await.unwrap();

    // Add collaborator user-collab with Collaborator role
    db.create_permission(RepositoryPermission {
        id: "perm-collab-1".to_string(),
        repo_id: "target-repo-1".to_string(),
        user_id: "user-collab".to_string(),
        role: PermissionRole::Collaborator,
        created_at_nanos: now,
    })
    .await
    .unwrap();

    // 1. Owner push -> Trusted
    let p1 = WorkflowProvenance::evaluate(db.as_ref(), "target-repo-1", None, "user-owner", "push")
        .await;
    assert_eq!(p1.trust_classification, TrustClassification::Trusted);
    assert!(p1.is_actor_authorized);
    assert!(!p1.is_fork);

    // 2. Collaborator internal PR -> Trusted
    let p2 = WorkflowProvenance::evaluate(
        db.as_ref(),
        "target-repo-1",
        Some("target-repo-1"),
        "user-collab",
        "pull_request",
    )
    .await;
    assert_eq!(p2.trust_classification, TrustClassification::Trusted);
    assert!(p2.is_actor_authorized);
    assert!(!p2.is_fork);

    // 3. External stranger PR -> Untrusted
    let p3 = WorkflowProvenance::evaluate(
        db.as_ref(),
        "target-repo-1",
        Some("target-repo-1"),
        "user-stranger",
        "pull_request",
    )
    .await;
    assert_eq!(p3.trust_classification, TrustClassification::Untrusted);
    assert!(!p3.is_actor_authorized);

    // 4. Fork PR (even from collaborator) -> Untrusted
    let p4 = WorkflowProvenance::evaluate(
        db.as_ref(),
        "target-repo-1",
        Some("fork-repo-99"),
        "user-collab",
        "pull_request",
    )
    .await;
    assert_eq!(p4.trust_classification, TrustClassification::Untrusted);
    assert!(p4.is_fork);

    // 5. Missing / non-existent repo -> Untrusted (fail closed)
    let p5 =
        WorkflowProvenance::evaluate(db.as_ref(), "non-existent-repo", None, "user-owner", "push")
            .await;
    assert_eq!(p5.trust_classification, TrustClassification::Untrusted);
}

#[tokio::test]
async fn test_ordering_defect_untrusted_job_not_leased_and_not_stranded() {
    use spine_api::core::types::*;
    use spine_api::db::Database;
    use spine_cluster::durability::DurabilityPolicy;

    let db = Arc::new(InMemoryDatabase::new());
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

    // Create target repo owned by user-owner
    let repo = ForgeRepository {
        id: "repo-order-1".to_string(),
        owner_user_id: "user-owner".to_string(),
        owner_username: "owner".to_string(),
        owner_org_id: None,
        name: "repo-order".to_string(),
        normalized_name: "repo-order".to_string(),
        description: "Test repo".to_string(),
        is_private: false,
        default_branch: "main".to_string(),
        native_repo_id: "repo-order-1".to_string(),
        durability_policy: DurabilityPolicy::ThreeCopies,
        provisioning_status: ProvisioningStatus::Ready,
        created_at_nanos: now,
        updated_at_nanos: now,
        archived: false,
    };
    db.create_repository(repo).await.unwrap();

    // Create a workflow run triggered by an untrusted external fork PR
    let run = WorkflowRun {
        id: "run-fork-1".to_string(),
        repo_id: "repo-order-1".to_string(),
        workflow_id: "wf-1".to_string(),
        workflow_name: "ci".to_string(),
        commit_oid: "0000000000000000000000000000000000000001".to_string(),
        ref_name: "refs/pull/1/merge".to_string(),
        trigger_event: "pull_request_fork".to_string(),
        actor_id: "untrusted-attacker".to_string(),
        status: "RUNNING".to_string(),
        conclusion: None,
        definition_yaml: "name: ci\non: [pull_request]\njobs:\n  test:\n    runs-on: [linux]\n"
            .to_string(),
        definition_digest: "digest1".to_string(),
        created_at_nanos: now,
        started_at_nanos: Some(now),
        completed_at_nanos: None,
    };
    db.create_workflow_run(run).await.unwrap();

    // Queue a job for this run
    let job = WorkflowJob {
        id: "job-fork-1".to_string(),
        run_id: "run-fork-1".to_string(),
        repo_id: "repo-order-1".to_string(),
        job_key: "test".to_string(),
        name: "test".to_string(),
        status: "QUEUED".to_string(),
        runner_id: None,
        runs_on: vec!["linux".to_string()],
        needs: vec![],
        attempt: 0,
        fencing_token: 0,
        lease_expires_at_nanos: None,
        exit_code: None,
        created_at_nanos: now,
        started_at_nanos: None,
        completed_at_nanos: None,
    };
    db.create_workflow_job(job).await.unwrap();

    let scheduler = WorkflowScheduler::new(db.clone(), None);
    let runner_svc =
        RunnerService::new(db.clone(), scheduler).with_profile(DeploymentProfile::CoreForge);

    // Runner attempts to acquire the job
    let acquired = runner_svc
        .acquire_job("runner-trusted-1", &["linux".to_string()])
        .await
        .expect("acquire_job execution");

    // Proves ordering defect fix:
    // 1. Runner must NOT acquire the untrusted job
    assert!(
        acquired.is_none(),
        "Runner must not acquire untrusted fork job under CoreForge"
    );

    // 2. The job must NOT be leased to runner-trusted-1 or stranded in RUNNING
    let job_after = db.get_workflow_job("job-fork-1").await.unwrap().unwrap();
    assert_ne!(
        job_after.status, "RUNNING",
        "Untrusted job must not be marked RUNNING or leased"
    );
    assert_eq!(
        job_after.status, "FAILED",
        "Untrusted job must fail closed cleanly"
    );
    assert_eq!(
        job_after.runner_id, None,
        "Untrusted job must have no assigned runner"
    );
    assert_eq!(
        job_after.exit_code,
        Some(1),
        "Failed untrusted job must have exit code 1"
    );
}
