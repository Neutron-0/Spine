use spine_cluster::failover::{FailoverCoordinator, FailoverError, ReplicaState};
use spine_cluster::fencing::{FencingError, OwnershipEpoch};
use spine_cluster::node::{FailureDomain, NodeCapacity, NodeStatus, StorageNodeInfo};

#[test]
fn test_fencing_token_rejects_stale_primary() {
    let current_epoch = OwnershipEpoch::new(18);

    // Old primary reconnects after partition and attempts write with epoch 17
    let write_attempt_epoch = 17u64;
    let res = current_epoch.validate(write_attempt_epoch);

    assert_eq!(
        res,
        Err(FencingError::StaleEpoch {
            request_epoch: 17,
            current_epoch: 18,
        })
    );

    // Authoritative primary with epoch 18 is permitted
    assert!(current_epoch.validate(18).is_ok());
}

#[test]
fn test_failover_eligibility_and_promotion_invariants() {
    let current_epoch = OwnershipEpoch::new(3);
    let committed_gen = 10u64;

    let nodes = vec![
        StorageNodeInfo {
            node_id: "node-b".into(),
            status: NodeStatus::Healthy,
            capacity: NodeCapacity::new(100_000, 10_000, 0),
            active_tasks: 1,
            ..StorageNodeInfo::new(
                "node-b".into(),
                "0b".repeat(32),
                "fp-b".into(),
                "127.0.0.1:9002".into(),
                "/data/b".into(),
                FailureDomain::default(),
                100_000,
                0,
            )
        },
        StorageNodeInfo {
            node_id: "node-c".into(),
            status: NodeStatus::Healthy,
            capacity: NodeCapacity::new(100_000, 10_000, 0),
            active_tasks: 0,
            ..StorageNodeInfo::new(
                "node-c".into(),
                "0c".repeat(32),
                "fp-c".into(),
                "127.0.0.1:9003".into(),
                "/data/c".into(),
                FailureDomain::default(),
                100_000,
                0,
            )
        },
        StorageNodeInfo {
            node_id: "node-d".into(),
            status: NodeStatus::Offline, // Offline node
            capacity: NodeCapacity::new(100_000, 10_000, 0),
            active_tasks: 0,
            ..StorageNodeInfo::new(
                "node-d".into(),
                "0d".repeat(32),
                "fp-d".into(),
                "127.0.0.1:9004".into(),
                "/data/d".into(),
                FailureDomain::default(),
                100_000,
                0,
            )
        },
    ];

    // Scenario 1: Only stale replicas available -> must fail failover!
    let stale_replicas = vec![
        ReplicaState {
            node_id: "node-b".into(),
            is_primary: false,
            last_synced_generation: 9, // Behind!
            verified: true,
            last_verified_at_nanos: 100,
        },
        ReplicaState {
            node_id: "node-c".into(),
            is_primary: false,
            last_synced_generation: 8, // Behind!
            verified: true,
            last_verified_at_nanos: 100,
        },
    ];

    let fail_res = FailoverCoordinator::evaluate_and_promote(
        "repo-alpha",
        "node-a",
        current_epoch,
        committed_gen,
        &stale_replicas,
        &nodes,
    );
    assert!(matches!(
        fail_res,
        Err(FailoverError::NoEligibleReplica { .. })
    ));

    // Scenario 2: Unverified replica -> cannot be promoted!
    let unverified_replicas = vec![ReplicaState {
        node_id: "node-b".into(),
        is_primary: false,
        last_synced_generation: 10,
        verified: false, // Bitrot detected or unverified!
        last_verified_at_nanos: 100,
    }];
    let fail_res = FailoverCoordinator::evaluate_and_promote(
        "repo-alpha",
        "node-a",
        current_epoch,
        committed_gen,
        &unverified_replicas,
        &nodes,
    );
    assert!(matches!(
        fail_res,
        Err(FailoverError::NoEligibleReplica { .. })
    ));

    // Scenario 3: Healthy, verified, up-to-date replica node-c (lowest active tasks) is promoted!
    let valid_replicas = vec![
        ReplicaState {
            node_id: "node-b".into(),
            is_primary: false,
            last_synced_generation: 10,
            verified: true,
            last_verified_at_nanos: 100,
        },
        ReplicaState {
            node_id: "node-c".into(),
            is_primary: false,
            last_synced_generation: 10,
            verified: true,
            last_verified_at_nanos: 100,
        },
    ];

    let promo = FailoverCoordinator::evaluate_and_promote(
        "repo-alpha",
        "node-a",
        current_epoch,
        committed_gen,
        &valid_replicas,
        &nodes,
    )
    .unwrap();

    assert_eq!(promo.repo_id, "repo-alpha");
    assert_eq!(promo.old_primary_id, "node-a");
    assert_eq!(promo.new_primary_id, "node-c"); // node-c had 0 active tasks vs 1 on node-b
    assert_eq!(promo.new_ownership_epoch, 4); // Monotonically incremented from 3 to 4!
    assert_eq!(promo.committed_generation, 10);
}
