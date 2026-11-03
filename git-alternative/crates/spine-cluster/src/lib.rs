pub mod durability;
pub mod enrollment;
pub mod failover;
pub mod fencing;
pub mod gc;
pub mod heartbeat;
pub mod node;
pub mod placement;
pub mod protocol;
pub mod repair;
pub mod store;
pub mod transaction;
pub mod write_pipeline;

pub use durability::{DurabilityError, DurabilityPolicy};
pub use enrollment::{EnrollmentError, EnrollmentRequest, EnrollmentResponse, EnrollmentToken};
pub use failover::{FailoverCoordinator, FailoverError, PromotionResult, ReplicaState};
pub use fencing::{FencingError, FencingToken, OwnershipEpoch};
pub use gc::{DistributedGcCoordinator, TombstoneRecord, TombstoneStatus};
pub use heartbeat::{evaluate_node_health, NodeHeartbeat};
pub use node::{FailureDomain, NodeCapacity, NodeError, NodeStatus, StorageNodeInfo};
pub use placement::{PlacementError, PlacementPlan, PlacementScheduler};
pub use protocol::{ClusterRpcRequest, ClusterRpcResponse};
pub use repair::{AntiEntropy, RepairJob, RepairStatus};
pub use store::{CoordinatorStore, FileBackedCoordinatorStore, RepositoryMeta};
pub use transaction::{ReplicationTransaction, TransactionError, TransactionPhase};
pub use write_pipeline::{
    AuthoritativeRefUpdate, AuthoritativeRepositoryWriter, AuthoritativeWriteResult,
    BranchProtectionRule, DurabilityVerificationReport, LocalBackendStorageDispatcher,
    NetworkStorageDispatcher, PrProtectionContext, StorageDispatcher, WriteActor,
    WritePipelineError,
};

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use rand::rngs::OsRng;
    use tempfile::tempdir;

    #[test]
    fn test_node_state_transitions() {
        let mut node = StorageNodeInfo::new(
            "node-1".into(),
            "00".repeat(32),
            "fp-1".into(),
            "127.0.0.1:9001".into(),
            "/data/node-1".into(),
            FailureDomain::default(),
            100_000_000,
            0,
        );

        assert_eq!(node.status, NodeStatus::Joining);
        assert!(node.transition_to(NodeStatus::Healthy).is_ok());
        assert_eq!(node.status, NodeStatus::Healthy);

        assert!(node.transition_to(NodeStatus::Suspect).is_ok());
        assert_eq!(node.status, NodeStatus::Suspect);

        assert!(node.transition_to(NodeStatus::Offline).is_ok());
        assert_eq!(node.status, NodeStatus::Offline);

        assert!(node.transition_to(NodeStatus::Retired).is_ok());
        assert_eq!(node.status, NodeStatus::Retired);

        // Terminal state cannot transition
        assert!(node.transition_to(NodeStatus::Healthy).is_err());
    }

    #[test]
    fn test_enrollment_token_and_signature_verification() {
        let cluster_id = "spine-prod-01";
        let token = EnrollmentToken::generate(cluster_id, 300, 1000);
        assert!(token.validate(cluster_id, 1001).is_ok());
        assert!(token.validate("other-cluster", 1001).is_err());
        assert!(token
            .validate(cluster_id, 1000 + 301 * 1_000_000_000)
            .is_err());

        // Signing key for node
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();
        let pk_hex = hex::encode(verifying_key.as_bytes());

        let message = format!("{}:{}:{}", token.token, cluster_id, pk_hex);
        let sig = signing_key.sign(message.as_bytes());
        let sig_hex = hex::encode(sig.to_bytes());

        let req = EnrollmentRequest {
            token: token.token.clone(),
            public_key_hex: pk_hex,
            listen_addr: "127.0.0.1:9100".into(),
            storage_root: "/data".into(),
            region: "us-east".into(),
            zone: "us-east-1a".into(),
            host: "storage-host-1".into(),
            total_capacity_bytes: 1_000_000_000,
            signature_hex: sig_hex,
        };

        assert!(req.verify_signature(cluster_id).is_ok());
        assert!(req.verify_signature("wrong-cluster").is_err());
    }

    #[test]
    fn test_durability_policies() {
        let single = DurabilityPolicy::SingleCopy;
        assert_eq!(single.required_copies(), 1);
        assert!(single.is_satisfied(1));
        assert!(single.verify_satisfaction(1).is_ok());
        assert!(single.verify_satisfaction(0).is_err());

        let two = DurabilityPolicy::TwoCopies;
        assert_eq!(two.required_copies(), 2);
        assert!(!two.is_satisfied(1));
        assert!(two.is_satisfied(2));

        let three = DurabilityPolicy::ThreeCopies;
        assert_eq!(three.required_copies(), 3);
        assert!(!three.is_satisfied(2));
        assert!(three.is_satisfied(3));
    }

    #[test]
    fn test_fencing_epoch_monotonicity() {
        let epoch = OwnershipEpoch::initial();
        assert_eq!(epoch.value(), 1);

        assert!(epoch.validate(1).is_ok());
        assert_eq!(
            epoch.validate(0),
            Err(FencingError::StaleEpoch {
                request_epoch: 0,
                current_epoch: 1
            })
        );
        assert_eq!(
            epoch.validate(2),
            Err(FencingError::EpochMismatch {
                request_epoch: 2,
                current_epoch: 1
            })
        );

        let next = epoch.next();
        assert_eq!(next.value(), 2);
        assert!(next.validate(2).is_ok());
    }

    #[test]
    fn test_placement_scheduler_diversity() {
        let n1 = StorageNodeInfo::new(
            "node-1".into(),
            "01".repeat(32),
            "fp-1".into(),
            "127.0.0.1:9001".into(),
            "/data/1".into(),
            FailureDomain {
                region: "us-east".into(),
                zone: "us-east-1a".into(),
                host: "host-1".into(),
            },
            1_000_000,
            0,
        );
        let n2 = StorageNodeInfo::new(
            "node-2".into(),
            "02".repeat(32),
            "fp-2".into(),
            "127.0.0.1:9002".into(),
            "/data/2".into(),
            FailureDomain {
                region: "us-east".into(),
                zone: "us-east-1b".into(),
                host: "host-2".into(),
            },
            2_000_000,
            0,
        );
        let n3 = StorageNodeInfo::new(
            "node-3".into(),
            "03".repeat(32),
            "fp-3".into(),
            "127.0.0.1:9003".into(),
            "/data/3".into(),
            FailureDomain {
                region: "us-east".into(),
                zone: "us-east-1c".into(),
                host: "host-3".into(),
            },
            3_000_000,
            0,
        );

        let mut nodes = vec![n1, n2, n3];
        for n in &mut nodes {
            n.status = NodeStatus::Healthy;
        }

        let plan = PlacementScheduler::schedule_placement(
            "repo-1",
            DurabilityPolicy::ThreeCopies,
            &nodes,
            500_000,
            1,
        )
        .unwrap();

        assert_eq!(plan.required_copies, 3);
        assert_eq!(plan.all_nodes().len(), 3);
        assert_eq!(plan.placement_generation, 2);
    }

    #[test]
    fn test_replication_transaction_2phase_lifecycle() {
        let mut tx = ReplicationTransaction::new(
            "tx-1".into(),
            "repo-1".into(),
            1,
            2,
            "node-1".into(),
            1,
            DurabilityPolicy::TwoCopies,
            vec!["node-1".into(), "node-2".into()],
        );

        assert_eq!(tx.phase, TransactionPhase::Initiated);
        assert!(tx.start_preparing().is_ok());
        assert_eq!(tx.phase, TransactionPhase::Preparing);

        // Ack primary
        assert!(!tx.acknowledge_prepared("node-1").unwrap());
        // Ack replica
        assert!(tx.acknowledge_prepared("node-2").unwrap());
        assert_eq!(tx.phase, TransactionPhase::Prepared);

        // Decide commit
        assert!(tx.decide_commit(100).is_ok());
        assert_eq!(tx.phase, TransactionPhase::CommitDecided);

        // Mark committed
        assert!(tx.mark_committed(200).is_ok());
        assert_eq!(tx.phase, TransactionPhase::Committed);
    }

    #[test]
    fn test_controlled_failover_and_stale_replica_rejection() {
        let nodes = vec![
            StorageNodeInfo {
                node_id: "node-2".into(),
                status: NodeStatus::Healthy,
                capacity: NodeCapacity::new(1000, 100, 0),
                active_tasks: 0,
                ..StorageNodeInfo::new(
                    "node-2".into(),
                    "02".repeat(32),
                    "fp-2".into(),
                    "127.0.0.1:9002".into(),
                    "/data/2".into(),
                    FailureDomain::default(),
                    1000,
                    0,
                )
            },
            StorageNodeInfo {
                node_id: "node-3".into(),
                status: NodeStatus::Healthy,
                capacity: NodeCapacity::new(1000, 100, 0),
                active_tasks: 0,
                ..StorageNodeInfo::new(
                    "node-3".into(),
                    "03".repeat(32),
                    "fp-3".into(),
                    "127.0.0.1:9003".into(),
                    "/data/3".into(),
                    FailureDomain::default(),
                    1000,
                    0,
                )
            },
        ];

        let replicas = vec![
            ReplicaState {
                node_id: "node-2".into(),
                is_primary: false,
                last_synced_generation: 5, // Up to date!
                verified: true,
                last_verified_at_nanos: 100,
            },
            ReplicaState {
                node_id: "node-3".into(),
                is_primary: false,
                last_synced_generation: 4, // Stale!
                verified: true,
                last_verified_at_nanos: 100,
            },
        ];

        let promo = FailoverCoordinator::evaluate_and_promote(
            "repo-1",
            "node-1",
            OwnershipEpoch(1),
            5,
            &replicas,
            &nodes,
        )
        .unwrap();

        assert_eq!(promo.new_primary_id, "node-2");
        assert_eq!(promo.new_ownership_epoch, 2);
    }

    #[test]
    fn test_anti_entropy_diff_packs() {
        let source = vec!["pack-1".into(), "pack-2".into(), "pack-3".into()];
        let target = vec!["pack-1".into()];
        let missing = AntiEntropy::diff_packs(&source, &target);
        assert_eq!(missing, vec!["pack-2".to_string(), "pack-3".to_string()]);
    }

    #[test]
    fn test_file_backed_store_persistence() {
        let dir = tempdir().unwrap();
        let store_path = dir.path().join("coord_state.json");

        {
            let store = FileBackedCoordinatorStore::new_file_backed(&store_path).unwrap();
            let mut node = StorageNodeInfo::new(
                "node-a".into(),
                "00".repeat(32),
                "fp-a".into(),
                "127.0.0.1:9001".into(),
                "/data/a".into(),
                FailureDomain::default(),
                5000,
                100,
            );
            node.status = NodeStatus::Healthy;
            store.register_node(node).unwrap();
        }

        // Reopen from disk
        {
            let store = FileBackedCoordinatorStore::new_file_backed(&store_path).unwrap();
            let node = store.get_node("node-a").unwrap().unwrap();
            assert_eq!(node.node_id, "node-a");
            assert_eq!(node.status, NodeStatus::Healthy);
        }
    }
}
