use spine_api::api::{build_router, AppState};
use spine_api::artifact::ArtifactService;
use spine_api::auth::mail::DevMailCatcher;
use spine_api::auth::pat::hash_pat;
use spine_api::auth::rate_limit::RateLimiter;
use spine_api::core::types::*;
use spine_api::db::{Database, InMemoryDatabase};
use spine_api::packages::{CargoRegistryAdapter, PackageService};
use spine_api::permissions::PermissionService;
use spine_api::releases::ReleaseService;
use spine_api::repository::RepositoryService;
use spine_api::storage::NativeVcsService;
use spine_artifact::LocalArtifactStore;
use std::fs;
use std::sync::Arc;
use tempfile::tempdir;
use tokio::net::TcpListener;
use tokio::process::Command;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_live_cargo_cli_publish_against_spine_api() {
    let tmp = tempdir().unwrap();
    let storage_root = tmp.path().join("storage");
    fs::create_dir_all(&storage_root).unwrap();

    let db = Arc::new(InMemoryDatabase::new());
    let perms = Arc::new(PermissionService::new(db.clone()));
    let _repos = Arc::new(RepositoryService::new(db.clone(), perms.clone()));
    let _vcs = Arc::new(NativeVcsService::new(storage_root.clone()));
    let _rate_limiter = Arc::new(RateLimiter::new());
    let _mail = Arc::new(DevMailCatcher::new());

    let artifact_store = Arc::new(LocalArtifactStore::new(storage_root.join("artifacts")).unwrap());
    let artifacts = Arc::new(ArtifactService::new(db.clone(), artifact_store));
    let _releases = Arc::new(ReleaseService::new(db.clone(), artifacts.clone()));
    let packages = Arc::new(PackageService::new(db.clone(), artifacts.clone()));
    let cargo = Arc::new(CargoRegistryAdapter::new(packages.clone()));

    // 1. Create a User and Personal Access Token (PAT)
    let user = User {
        id: "usr_cargo_dev".to_string(),
        username: "cargodev".to_string(),
        normalized_username: "cargodev".to_string(),
        email: "cargodev@spine.build".to_string(),
        normalized_email: "cargodev@spine.build".to_string(),
        display_name: "Cargo Dev".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: 1,
        updated_at_nanos: 1,
        last_login_nanos: None,
    };
    db.create_user(user.clone()).await.unwrap();

    let pat_secret = "spine_pat_live_test_1234567890abcdef";
    let token_hash = hash_pat(pat_secret);
    let pat = PersonalAccessToken {
        id: "pat_1".to_string(),
        user_id: user.id.clone(),
        name: "Cargo Publish Token".to_string(),
        token_hash,
        scopes: vec!["repo".to_string(), "package".to_string()],
        expires_at_nanos: i64::MAX,
        created_at_nanos: 1,
        last_used_at_nanos: None,
        revoked: false,
    };
    db.create_pat(pat).await.unwrap();

    let state = AppState::new_with_defaults(db.clone(), &storage_root, None).unwrap();

    let router = build_router(state);

    // 2. Bind ephemeral server
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();

    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    // 3. Create dummy crate in tempdir
    let crate_dir = tmp.path().join("spine_test_crate");
    let src_dir = crate_dir.join("src");
    let dot_cargo = crate_dir.join(".cargo");
    fs::create_dir_all(&src_dir).unwrap();
    fs::create_dir_all(&dot_cargo).unwrap();

    let cargo_toml = r#"
[package]
name = "spine-test-crate"
version = "0.1.0"
edition = "2021"
license = "MIT"
description = "A real crate published to Spine Forge via live cargo CLI"

[lib]
path = "src/lib.rs"
"#;
    fs::write(crate_dir.join("Cargo.toml"), cargo_toml).unwrap();
    fs::write(
        src_dir.join("lib.rs"),
        "pub fn spine_hello() -> &'static str { \"Hello from Spine Registry!\" }\n",
    )
    .unwrap();

    // 4. Configure .cargo/config.toml with sparse index
    let config_toml = format!(
        r#"
[registries.spine-live]
index = "sparse+http://127.0.0.1:{}/index/"
"#,
        port
    );
    fs::write(dot_cargo.join("config.toml"), config_toml).unwrap();

    // 5. Execute actual `cargo publish` CLI command!
    let output = Command::new("cargo")
        .args([
            "publish",
            "--registry",
            "spine-live",
            "--no-verify",
            "--allow-dirty",
        ])
        .current_dir(&crate_dir)
        .stdin(std::process::Stdio::null())
        .env("CARGO_REGISTRIES_SPINE_LIVE_TOKEN", pat_secret)
        .output()
        .await
        .expect("Failed to execute cargo command");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    println!("cargo stdout: {}", stdout);
    println!("cargo stderr: {}", stderr);

    assert!(
        output.status.success(),
        "cargo publish should exit successfully! Stderr: {}",
        stderr
    );

    // 6. Verify via CargoRegistryAdapter that package was published and indexed
    let index_file = cargo
        .get_index_file("spine-test-crate")
        .await
        .expect("Index fetch should succeed")
        .expect("Index should contain spine-test-crate");

    assert!(index_file.contains("\"name\":\"spine-test-crate\""));
    assert!(index_file.contains("\"vers\":\"0.1.0\""));
    assert!(index_file.contains("\"cksum\":"));

    // 7. Verify crate download
    let downloaded_crate = cargo
        .download("spine-test-crate", "0.1.0")
        .await
        .expect("Crate download should succeed");
    assert!(!downloaded_crate.is_empty());

    // 8. Attempt overwrite: Publishing the same version again MUST be rejected with 409 Conflict
    let overwrite_output = Command::new("cargo")
        .args([
            "publish",
            "--registry",
            "spine-live",
            "--no-verify",
            "--allow-dirty",
        ])
        .current_dir(&crate_dir)
        .stdin(std::process::Stdio::null())
        .env("CARGO_REGISTRIES_SPINE_LIVE_TOKEN", pat_secret)
        .output()
        .await
        .expect("Failed to execute cargo overwrite attempt");

    let overwrite_stderr = String::from_utf8_lossy(&overwrite_output.stderr);
    println!("overwrite stderr: {}", overwrite_stderr);
    assert!(
        !overwrite_output.status.success(),
        "Re-publishing an existing version MUST fail with error"
    );
    assert!(
        overwrite_stderr.contains("already exists")
            || overwrite_stderr.contains("409")
            || overwrite_stderr.contains("already been published"),
        "Overwrite should report that crate version already exists: {}",
        overwrite_stderr
    );

    // 9. Consumer project: create an independent project consuming spine-test-crate from Spine registry
    let consumer_dir = tmp.path().join("spine_consumer_app");
    let consumer_src = consumer_dir.join("src");
    let consumer_cargo = consumer_dir.join(".cargo");
    fs::create_dir_all(&consumer_src).unwrap();
    fs::create_dir_all(&consumer_cargo).unwrap();

    let consumer_cargo_toml = r#"
[package]
name = "spine-consumer-app"
version = "0.1.0"
edition = "2021"

[dependencies]
spine-test-crate = { version = "0.1.0", registry = "spine-live" }
"#
    .to_string();
    fs::write(consumer_dir.join("Cargo.toml"), consumer_cargo_toml).unwrap();
    fs::write(
        consumer_src.join("main.rs"),
        "fn main() { println!(\"{}\", spine_test_crate::spine_hello()); }\n",
    )
    .unwrap();

    let consumer_config = format!(
        r#"
[registries.spine-live]
index = "sparse+http://127.0.0.1:{}/index/"
credential-provider = "cargo:token"
"#,
        port
    );
    fs::write(consumer_cargo.join("config.toml"), consumer_config).unwrap();

    // 10. Run `cargo check` in the consumer project to verify resolution, download, and compilation!
    let check_output = Command::new("cargo")
        .args(["check"])
        .current_dir(&consumer_dir)
        .stdin(std::process::Stdio::null())
        .env("CARGO_REGISTRIES_SPINE_LIVE_TOKEN", pat_secret)
        .output()
        .await
        .expect("Failed to execute cargo check on consumer project");

    let check_stdout = String::from_utf8_lossy(&check_output.stdout);
    let check_stderr = String::from_utf8_lossy(&check_output.stderr);
    println!("consumer check stdout: {}", check_stdout);
    println!("consumer check stderr: {}", check_stderr);

    assert!(
        check_output.status.success(),
        "Consumer project must download and build successfully! Stderr: {}",
        check_stderr
    );
}
