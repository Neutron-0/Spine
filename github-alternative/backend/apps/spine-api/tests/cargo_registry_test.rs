use sha2::{Digest, Sha256};
use spine_api::artifact::ArtifactService;
use spine_api::db::InMemoryDatabase;
use spine_api::packages::{CargoRegistryAdapter, PackageService};
use spine_artifact::LocalArtifactStore;
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn test_cargo_registry_sparse_index_and_publish_protocol() {
    let tmp = tempdir().unwrap();
    let db = Arc::new(InMemoryDatabase::new());
    let store = Arc::new(LocalArtifactStore::new(tmp.path().join("artifacts")).unwrap());
    let artifacts = Arc::new(ArtifactService::new(db.clone(), store));
    let packages = Arc::new(PackageService::new(db.clone(), artifacts));
    let cargo = CargoRegistryAdapter::new(packages.clone());

    // 1. Verify prefix calculation for Cargo sparse index layout
    assert_eq!(CargoRegistryAdapter::crate_prefix("a"), "1");
    assert_eq!(CargoRegistryAdapter::crate_prefix("ab"), "2");
    assert_eq!(CargoRegistryAdapter::crate_prefix("abc"), "3/a");
    assert_eq!(CargoRegistryAdapter::crate_prefix("spine-artifact"), "sp/in");

    // 2. Verify sparse index config JSON
    let config = CargoRegistryAdapter::config_json("http://127.0.0.1:3000");
    assert_eq!(
        config["dl"],
        "http://127.0.0.1:3000/api/v1/crates/{crate}/{version}/download"
    );
    assert_eq!(config["api"], "http://127.0.0.1:3000");

    // 3. Construct a genuine Cargo publish binary payload
    let meta_json = serde_json::json!({
        "name": "spine-test-crate",
        "vers": "0.1.0",
        "deps": [
            {
                "name": "serde",
                "version_req": "^1.0",
                "features": ["derive"],
                "optional": false,
                "default_features": true,
                "target": null,
                "kind": "normal"
            }
        ],
        "features": {},
        "authors": ["Spine Project Engineers"],
        "description": "Test crate for Spine Cargo Registry validation",
        "license": "MIT"
    });

    let meta_bytes = serde_json::to_vec(&meta_json).unwrap();
    let fake_crate_tarball = b"GZIP_TARBALL_CRATE_PAYLOAD_SPINE_NATIVE_123456789";

    let mut payload = Vec::new();
    // 4 bytes LE JSON len
    payload.extend_from_slice(&(meta_bytes.len() as u32).to_le_bytes());
    // Metadata JSON
    payload.extend_from_slice(&meta_bytes);
    // 4 bytes LE crate len
    payload.extend_from_slice(&(fake_crate_tarball.len() as u32).to_le_bytes());
    // Tarball bytes
    payload.extend_from_slice(fake_crate_tarball);

    // Compute expected SHA256 checksum
    let mut hasher = Sha256::new();
    hasher.update(fake_crate_tarball);
    let expected_sha256 = hex::encode(hasher.finalize());

    // 4. Publish crate via adapter
    let user_id = "user_dev";
    let resp = cargo
        .publish(&payload, user_id)
        .await
        .expect("Cargo publish should succeed");
    assert!(resp.warnings.invalid_categories.is_empty());

    // 5. Query sparse index file
    let index_content = cargo
        .get_index_file("spine-test-crate")
        .await
        .expect("Index fetch should succeed")
        .expect("Index file should exist for published crate");

    let lines: Vec<&str> = index_content.lines().collect();
    assert_eq!(lines.len(), 1);

    let index_entry: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(index_entry["name"], "spine-test-crate");
    assert_eq!(index_entry["vers"], "0.1.0");
    assert_eq!(index_entry["cksum"], expected_sha256);
    assert_eq!(index_entry["yanked"], false);
    assert_eq!(index_entry["deps"].as_array().unwrap().len(), 1);

    // 6. Download .crate and verify contents & checksum
    let downloaded_crate = cargo
        .download("spine-test-crate", "0.1.0")
        .await
        .expect("Download should succeed");
    assert_eq!(downloaded_crate, fake_crate_tarball);

    // 7. Test Yank and Unyank
    cargo
        .set_yanked("spine-test-crate", "0.1.0", true)
        .await
        .expect("Yank should succeed");

    let yanked_index = cargo.get_index_file("spine-test-crate").await.unwrap().unwrap();
    let yanked_entry: serde_json::Value = serde_json::from_str(yanked_index.lines().next().unwrap()).unwrap();
    assert_eq!(yanked_entry["yanked"], true);

    cargo
        .set_yanked("spine-test-crate", "0.1.0", false)
        .await
        .expect("Unyank should succeed");

    let unyanked_index = cargo.get_index_file("spine-test-crate").await.unwrap().unwrap();
    let unyanked_entry: serde_json::Value = serde_json::from_str(unyanked_index.lines().next().unwrap()).unwrap();
    assert_eq!(unyanked_entry["yanked"], false);

    // 8. Immutability: Attempting to publish same version again must fail
    let dup_err = cargo.publish(&payload, user_id).await;
    assert!(dup_err.is_err());
}
