use axum::body::Body;
use axum::http::{Request, StatusCode};
use std::collections::HashMap;
use std::fs;
use std::sync::Arc;
use tempfile::tempdir;
use tower::ServiceExt;

use spine_api::{
    api::build_router,
    core::types::*,
    db::{Database, InMemoryDatabase},
    runner::executor::{IsolatedRunnerExecutor, RunnerSandboxMode},
    security::service::{compute_keyed_safe_fingerprint, compute_safe_fingerprint},
    webhook::ssrf::SsrfValidator,
    AppState,
};

#[tokio::test]
async fn test_runner_isolation_fail_closed_on_unsandboxed_untrusted_workflow() {
    let tmp = tempdir().unwrap();
    let executor = IsolatedRunnerExecutor::new(tmp.path().to_path_buf())
        .with_sandbox_mode(RunnerSandboxMode::Unsandboxed);

    let job = WorkflowJob {
        id: "job-untrusted-01".to_string(),
        run_id: "run-01".to_string(),
        repo_id: "repo-public-fork".to_string(),
        job_key: "build".to_string(),
        name: "Untrusted Build Step".to_string(),
        status: "RUNNING".to_string(),
        runner_id: Some("runner-01".to_string()),
        runs_on: vec!["windows".to_string()],
        needs: vec![],
        attempt: 1,
        fencing_token: 1,
        lease_expires_at_nanos: Some(i64::MAX),
        exit_code: None,
        created_at_nanos: 1,
        started_at_nanos: Some(1),
        completed_at_nanos: None,
    };

    let step = WorkflowStep {
        id: "step-01".to_string(),
        job_id: job.id.clone(),
        step_index: 1,
        name: "Malicious Step".to_string(),
        status: "QUEUED".to_string(),
        exit_code: None,
        started_at_nanos: None,
        completed_at_nanos: None,
    };

    let mut scripts = HashMap::new();
    scripts.insert(1, "echo trying to access host".to_string());

    // 1. Untrusted execution with Unsandboxed mode MUST FAIL CLOSED
    let (exit_code, results) = executor
        .execute_job_with_trust(&job, &[step.clone()], &scripts, &[], true)
        .await;

    assert_eq!(
        exit_code, 1,
        "Untrusted execution without sandbox must fail closed"
    );
    assert_eq!(results.len(), 1);
    assert!(
        results[0]
            .stderr
            .contains("SECURITY ERROR: Untrusted workflow execution requires a verified sandbox"),
        "Must return explicit security error explaining sandbox requirement"
    );

    // 2. Trusted local development mode allows local run
    let trusted_executor = IsolatedRunnerExecutor::new(tmp.path().to_path_buf())
        .with_sandbox_mode(RunnerSandboxMode::TrustedLocalDevelopment);

    let (trusted_exit, trusted_res) = trusted_executor
        .execute_job_with_trust(&job, &[step], &scripts, &[], false)
        .await;

    assert_eq!(trusted_exit, 0, "Trusted local execution should succeed");
    assert_eq!(trusted_res.len(), 1);
    assert_eq!(trusted_res[0].exit_code, 0);
}

#[tokio::test]
async fn test_stale_fencing_token_and_runner_lease_invalidation() {
    let tmp = tempdir().unwrap();
    let storage_root = tmp.path().join("storage");
    fs::create_dir_all(&storage_root).unwrap();

    let db = Arc::new(InMemoryDatabase::new());
    let state = AppState::new_with_defaults(db.clone(), &storage_root, None).unwrap();

    let (runner, _token) = state
        .runners
        .register_runner(
            "worker-1",
            "host-1",
            "windows",
            "x86_64",
            vec!["windows".to_string()],
            2,
        )
        .await
        .unwrap();

    let job = WorkflowJob {
        id: "job-lease-01".to_string(),
        run_id: "run-01".to_string(),
        repo_id: "repo-01".to_string(),
        job_key: "test".to_string(),
        name: "Test Job".to_string(),
        status: "QUEUED".to_string(),
        runner_id: None,
        runs_on: vec!["windows".to_string()],
        needs: vec![],
        attempt: 1,
        fencing_token: 10,
        lease_expires_at_nanos: None,
        exit_code: None,
        created_at_nanos: 1,
        started_at_nanos: None,
        completed_at_nanos: None,
    };
    db.create_workflow_job(job.clone()).await.unwrap();

    // Acquire job -> increments fencing_token to 11
    let acquired = state
        .runners
        .acquire_job(&runner.id, &["windows".to_string()])
        .await
        .unwrap();
    assert!(acquired.is_some());
    let (assigned_job, _, _) = acquired.unwrap();
    assert_eq!(assigned_job.fencing_token, 11);

    // Stale fencing token (10 instead of 11) must be rejected
    let stale_log = state
        .runners
        .ingest_logs(&runner.id, &assigned_job.id, 10, 1, "stdout", "test log")
        .await;
    assert!(stale_log.is_err(), "Stale fencing token must be rejected");

    let stale_complete = state
        .runners
        .complete_job(&runner.id, &assigned_job.id, 10, 0, &[(1, 0)])
        .await;
    assert!(
        stale_complete.is_err(),
        "Completion with stale fencing token must be rejected"
    );

    // Impostor runner ID must be rejected
    let impostor_complete = state
        .runners
        .complete_job("fake-runner-id", &assigned_job.id, 11, 0, &[(1, 0)])
        .await;
    assert!(
        impostor_complete.is_err(),
        "Completion by wrong runner identity must be rejected"
    );

    // Valid fencing token and runner identity succeeds
    let valid_complete = state
        .runners
        .complete_job(&runner.id, &assigned_job.id, 11, 0, &[(1, 0)])
        .await;
    assert!(
        valid_complete.is_ok(),
        "Completion with matching lease and token must succeed"
    );
}

#[tokio::test]
async fn test_secret_fingerprinting_keyed_hmac_and_no_plaintext_leakage() {
    let raw_secret = "sp_pat_9876543210abcdef9876543210fedcba";

    let fp1 = compute_safe_fingerprint(raw_secret);
    assert!(
        !fp1.contains(raw_secret),
        "Plaintext must never appear in safe fingerprint"
    );
    assert!(fp1.starts_with("sp_p"));
    assert!(fp1.contains("...****..."));
    assert!(fp1.contains("(hmac-sha256:v1:"));
    assert!(fp1.ends_with(')'));

    // Key rotation: computing with v2 key yields different keyed digest
    let v2_key = b"rotated_spine_salt_key_version_2_2026";
    let fp2 = compute_keyed_safe_fingerprint(raw_secret, "hmac-sha256:v2", v2_key);
    assert!(fp2.contains("hmac-sha256:v2:"));
    assert_ne!(
        fp1, fp2,
        "Rotated key must generate versioned distinct hash"
    );
}

#[tokio::test]
async fn test_ssrf_validator_blocks_internal_addresses_and_prohibited_ports() {
    let validator = SsrfValidator::new(false);

    // Forbidden IPs
    let forbidden_ips = vec![
        "127.0.0.1",
        "10.0.0.1",
        "172.16.0.1",
        "192.168.1.1",
        "169.254.169.254", // Cloud metadata
        "::1",
        "::ffff:127.0.0.1", // IPv4-mapped IPv6
    ];

    for ip_str in forbidden_ips {
        let ip: std::net::IpAddr = ip_str.parse().unwrap();
        let res = validator.validate_ip(ip);
        assert!(res.is_err(), "IP {} must be blocked by SSRF policy", ip_str);
    }

    // Forbidden ports
    let forbidden_urls = vec![
        "http://example.com:5432/webhook", // PostgreSQL
        "http://example.com:3306/webhook", // MySQL
        "http://example.com:6379/webhook", // Redis
        "http://example.com:22/webhook",   // SSH
    ];

    for url_str in forbidden_urls {
        let res = validator.validate_and_resolve(url_str).await;
        assert!(
            res.is_err(),
            "Prohibited port in URL {} must be blocked",
            url_str
        );
    }
}

#[tokio::test]
async fn test_observability_liveness_readiness_and_diagnostics_discrimination() {
    let tmp = tempdir().unwrap();
    let storage_root = tmp.path().join("storage");
    fs::create_dir_all(&storage_root).unwrap();

    let db = Arc::new(InMemoryDatabase::new());
    let state = AppState::new_with_defaults(db.clone(), &storage_root, None).unwrap();
    let app = build_router(state);

    // 1. Liveness check: always 200 OK
    let live_req = Request::builder()
        .uri("/health/live")
        .body(Body::empty())
        .unwrap();
    let live_res = app.clone().oneshot(live_req).await.unwrap();
    assert_eq!(live_res.status(), StatusCode::OK);

    // 2. Readiness check: 200 OK with connected DB
    let ready_req = Request::builder()
        .uri("/health/ready")
        .body(Body::empty())
        .unwrap();
    let ready_res = app.clone().oneshot(ready_req).await.unwrap();
    assert_eq!(ready_res.status(), StatusCode::OK);

    // 3. Structured Diagnostics check
    let diag_req = Request::builder()
        .uri("/health/diagnostics")
        .body(Body::empty())
        .unwrap();
    let diag_res = app.clone().oneshot(diag_req).await.unwrap();
    assert_eq!(diag_res.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(diag_res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["status"], "HEALTHY");
    assert_eq!(json["database_connected"], true);
    assert_eq!(json["service"], "spine-forge-api");
    assert!(json.get("timestamp_nanos").is_some());
}
