use spine_api::core::types::*;
use spine_api::db::{Database, InMemoryDatabase, PgDatabase};
use spine_api::workflow::WorkflowScheduler;
use std::sync::Arc;

#[tokio::test]
async fn test_workflow_scheduler_does_not_starve_older_runs() {
    let db: Arc<dyn Database> = Arc::new(InMemoryDatabase::new());
    let scheduler = WorkflowScheduler::new(Arc::clone(&db), None);
    let repo_id = "repo_starve_test";

    // 1. Create an old active workflow run
    let old_run = WorkflowRun {
        id: "run_old_active".to_string(),
        repo_id: repo_id.to_string(),
        workflow_id: "wf_1".to_string(),
        workflow_name: "CI".to_string(),
        commit_oid: "c1".to_string(),
        ref_name: "refs/heads/main".to_string(),
        trigger_event: "push".to_string(),
        actor_id: "user_1".to_string(),
        status: "PENDING".to_string(),
        conclusion: None,
        definition_yaml: "name: CI\non: push\njobs:\n  build:\n    runs-on: spine-runner\n    steps:\n      - run: echo ok".to_string(),
        definition_digest: "digest1".to_string(),
        source_repo_id: None,
        is_fork: false,
        trust_classification: TrustClassification::Trusted,
        provenance_reason: "trusted".to_string(),
        created_at_nanos: 100_000,
        started_at_nanos: None,
        completed_at_nanos: None,
    };
    db.create_workflow_run(old_run).await.unwrap();

    let job = WorkflowJob {
        id: "job_old_1".to_string(),
        run_id: "run_old_active".to_string(),
        repo_id: repo_id.to_string(),
        job_key: "build".to_string(),
        name: "Build".to_string(),
        status: "SUCCEEDED".to_string(),
        runner_id: Some("runner_1".to_string()),
        runs_on: vec!["spine-runner".to_string()],
        needs: vec![],
        attempt: 1,
        fencing_token: 1,
        lease_expires_at_nanos: None,
        exit_code: Some(0),
        created_at_nanos: 100_000,
        started_at_nanos: Some(100_000),
        completed_at_nanos: Some(100_500),
    };
    db.create_workflow_job(job).await.unwrap();

    // 2. Create 110 newer runs (completed)
    for i in 1..=110 {
        let run = WorkflowRun {
            id: format!("run_newer_{}", i),
            repo_id: repo_id.to_string(),
            workflow_id: "wf_1".to_string(),
            workflow_name: "CI".to_string(),
            commit_oid: format!("c{}", i),
            ref_name: "refs/heads/main".to_string(),
            trigger_event: "push".to_string(),
            actor_id: "user_1".to_string(),
            status: "COMPLETED".to_string(),
            conclusion: Some("SUCCESS".to_string()),
            definition_yaml: "".to_string(),
            definition_digest: "".to_string(),
            source_repo_id: None,
            is_fork: false,
            trust_classification: TrustClassification::Trusted,
            provenance_reason: "trusted".to_string(),
            created_at_nanos: 200_000 + i as i64 * 1000,
            started_at_nanos: Some(200_000),
            completed_at_nanos: Some(200_500),
        };
        db.create_workflow_run(run).await.unwrap();
    }

    // 3. Tick the scheduler for this repo
    scheduler.tick(repo_id).await.unwrap();

    // 4. Verify that old_run was advanced and not starved out by the 110 newer runs!
    let updated_old = db
        .get_workflow_run("run_old_active")
        .await
        .unwrap()
        .expect("run must exist");
    assert_ne!(
        updated_old.status, "PENDING",
        "Older active run must be picked up and transitioned by scheduler, not starved"
    );
}

#[test]
fn test_spine_db_pool_settings_from_env() {
    std::env::set_var("SPINE_DB_MAX_CONNECTIONS", "75");
    std::env::set_var("SPINE_DB_MIN_CONNECTIONS", "15");
    std::env::set_var("SPINE_DB_TIMEOUT_SECONDS", "45");

    let settings = PgDatabase::pool_settings_from_env();
    assert_eq!(settings.max_connections, 75);
    assert_eq!(settings.min_connections, 15);
    assert_eq!(settings.acquire_timeout_secs, 45);

    // Clean up
    std::env::remove_var("SPINE_DB_MAX_CONNECTIONS");
    std::env::remove_var("SPINE_DB_MIN_CONNECTIONS");
    std::env::remove_var("SPINE_DB_TIMEOUT_SECONDS");
}
