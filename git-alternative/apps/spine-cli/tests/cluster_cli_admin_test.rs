use spine_cluster::durability::DurabilityPolicy;
use spine_cluster::node::{FailureDomain, NodeStatus, StorageNodeInfo};
use spine_cluster::placement::PlacementPlan;
use spine_cluster::store::{CoordinatorStore, FileBackedCoordinatorStore, RepositoryMeta};
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_cluster_cli_admin_commands() {
    let spine_bin = env!("CARGO_BIN_EXE_spine");
    let temp = tempdir().unwrap();
    let root = temp.path();

    let coord_file = root.join(".spine-coordinator-state.json");
    let store = FileBackedCoordinatorStore::new_file_backed(&coord_file).unwrap();

    let mut n1 = StorageNodeInfo::new(
        "node-east-1".into(),
        "01".repeat(32),
        "fp:node-east-1".into(),
        "127.0.0.1:9101".into(),
        root.join("data-1").to_string_lossy().to_string(),
        FailureDomain {
            region: "us-east".into(),
            zone: "us-east-1a".into(),
            host: "host-1".into(),
        },
        50_000_000_000,
        1000,
    );
    n1.status = NodeStatus::Healthy;
    store.register_node(n1).unwrap();

    let repo = RepositoryMeta {
        repo_id: "org/project-x".into(),
        name: "project-x".into(),
        owner: "org".into(),
        durability_policy: DurabilityPolicy::TwoCopies,
        committed_generation: 1,
        current_ownership_epoch: 1,
        storage_quota_bytes: 10_000_000_000,
        state: "HEALTHY".into(),
        created_at_nanos: 1000,
        updated_at_nanos: 1000,
    };
    store.create_repository(repo).unwrap();

    let placement = PlacementPlan {
        repo_id: "org/project-x".into(),
        primary_node_id: "node-east-1".into(),
        replica_node_ids: vec![],
        required_copies: 2,
        placement_generation: 1,
    };
    store.set_placement(placement).unwrap();

    // 1. Test `spine cluster status`
    let out = Command::new(spine_bin)
        .args(["cluster", "status"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("SPINE CLUSTER STATUS"));
    assert!(stdout.contains("Total Storage Nodes:    1"));
    assert!(stdout.contains("1 healthy"));

    // 2. Test `spine node list`
    let out = Command::new(spine_bin)
        .args(["node", "list"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("node-east-1"));
    assert!(stdout.contains("HEALTHY"));
    assert!(stdout.contains("127.0.0.1:9101"));

    // 3. Test `spine node inspect node-east-1`
    let out = Command::new(spine_bin)
        .args(["node", "inspect", "node-east-1"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Node ID:          node-east-1"));
    assert!(stdout.contains("Failure Domain:   region=us-east, zone=us-east-1a, host=host-1"));

    // 4. Test `spine storage placements`
    let out = Command::new(spine_bin)
        .args(["storage", "placements"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("org/project-x"));
    assert!(stdout.contains("node-east-1"));
    assert!(stdout.contains("TWO_COPIES"));

    // 5. Test `spine replication status`
    let out = Command::new(spine_bin)
        .args(["replication", "status"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Active Replication Transactions:"));

    // 6. Test `spine node drain` and `spine node quarantine`
    let out = Command::new(spine_bin)
        .args(["node", "drain", "node-east-1"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("DRAINING"));

    let out = Command::new(spine_bin)
        .args(["node", "quarantine", "node-east-1"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("QUARANTINED"));
}
