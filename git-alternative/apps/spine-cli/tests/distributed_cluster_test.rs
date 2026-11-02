use ed25519_dalek::{Signer, SigningKey};
use rand::rngs::OsRng;
use spine_cluster::durability::DurabilityPolicy;
use spine_cluster::enrollment::{EnrollmentRequest, EnrollmentToken};
use spine_cluster::failover::FailoverCoordinator;
use spine_cluster::node::{FailureDomain, NodeStatus, StorageNodeInfo};
use spine_cluster::placement::PlacementScheduler;
use spine_cluster::store::{CoordinatorStore, FileBackedCoordinatorStore, RepositoryMeta};
use spine_cluster::transaction::{ReplicationTransaction, TransactionPhase};
use std::fs;
use tempfile::tempdir;

#[test]
fn test_multi_node_cluster_full_lifecycle() {
    let temp = tempdir().unwrap();
    let root = temp.path();

    // 1. Initialize coordinator store
    let coord_file = root.join("coordinator.json");
    let store = FileBackedCoordinatorStore::new_file_backed(&coord_file).unwrap();
    let cluster_id = "test-cluster-01";

    // 2. Generate enrollment tokens for 3 storage nodes
    let now = 1000i64;
    let tok1 = EnrollmentToken::generate(cluster_id, 3600, now);
    let tok2 = EnrollmentToken::generate(cluster_id, 3600, now);
    let tok3 = EnrollmentToken::generate(cluster_id, 3600, now);

    store.create_token(tok1.clone()).unwrap();
    store.create_token(tok2.clone()).unwrap();
    store.create_token(tok3.clone()).unwrap();

    // 3. Enroll 3 storage nodes with cryptographic Ed25519 identities
    let mut node_keys = Vec::new();
    let mut node_infos = Vec::new();

    let tokens = [tok1, tok2, tok3];
    for (i, tok) in tokens.iter().enumerate() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let pk = signing_key.verifying_key();
        let pk_hex = hex::encode(pk.as_bytes());

        let msg = format!("{}:{}:{}", tok.token, cluster_id, pk_hex);
        let sig = signing_key.sign(msg.as_bytes());
        let sig_hex = hex::encode(sig.to_bytes());

        let node_id = format!("node-{}", i + 1);
        let listen = format!("127.0.0.1:{}", 9100 + i);
        let node_dir = root.join(&node_id);
        fs::create_dir_all(&node_dir).unwrap();

        let req = EnrollmentRequest {
            token: tok.token.clone(),
            public_key_hex: pk_hex.clone(),
            listen_addr: listen.clone(),
            storage_root: node_dir.to_string_lossy().to_string(),
            region: "us-east".into(),
            zone: format!("us-east-1{}", (b'a' + i as u8) as char),
            host: format!("host-{}", i + 1),
            total_capacity_bytes: 10_000_000_000,
            signature_hex: sig_hex,
        };

        assert!(req.verify_signature(cluster_id).is_ok());
        store.consume_token(&tok.token).unwrap();

        let mut info = StorageNodeInfo::new(
            node_id.clone(),
            pk_hex,
            format!("fp:{}", node_id),
            listen,
            node_dir.to_string_lossy().to_string(),
            FailureDomain {
                region: req.region,
                zone: req.zone,
                host: req.host,
            },
            req.total_capacity_bytes,
            now,
        );
        info.status = NodeStatus::Healthy;
        store.register_node(info.clone()).unwrap();

        node_keys.push(signing_key);
        node_infos.push(info);
    }

    let enrolled_nodes = store.list_nodes().unwrap();
    assert_eq!(enrolled_nodes.len(), 3);
    assert!(enrolled_nodes
        .iter()
        .all(|n| n.status == NodeStatus::Healthy));

    // 4. Create repository with THREE_COPIES durability policy
    let repo_id = "spine/game-engine";
    let repo_meta = RepositoryMeta {
        repo_id: repo_id.into(),
        name: "game-engine".into(),
        owner: "spine".into(),
        durability_policy: DurabilityPolicy::ThreeCopies,
        committed_generation: 0,
        current_ownership_epoch: 1,
        storage_quota_bytes: 50_000_000_000,
        state: "HEALTHY".into(),
        created_at_nanos: now,
        updated_at_nanos: now,
    };
    store.create_repository(repo_meta).unwrap();

    // 5. Schedule placement across nodes
    let placement = PlacementScheduler::schedule_placement(
        repo_id,
        DurabilityPolicy::ThreeCopies,
        &enrolled_nodes,
        1_000_000,
        0,
    )
    .unwrap();

    assert_eq!(placement.required_copies, 3);
    assert_eq!(placement.all_nodes().len(), 3);
    let primary_id = placement.primary_node_id.clone();
    store.set_placement(placement.clone()).unwrap();

    // Initialize replicas in store
    for node_id in placement.all_nodes() {
        store
            .update_replica_state(repo_id, &node_id, node_id == primary_id, 0, true, now)
            .unwrap();
    }

    // 6. Execute 2-Phase Replication Transaction
    let pack_data = b"SPK1\x00\x00\x00\x00FAKE_PACK_CONTENT_OBJECTS";
    let pack_checksum = blake3::hash(pack_data).to_hex().to_string();

    let mut tx = ReplicationTransaction::new(
        "tx-push-001".into(),
        repo_id.into(),
        0,
        1,
        primary_id.clone(),
        1,
        DurabilityPolicy::ThreeCopies,
        placement.all_nodes(),
    );

    assert_eq!(tx.phase, TransactionPhase::Initiated);
    tx.start_preparing().unwrap();
    assert_eq!(tx.phase, TransactionPhase::Preparing);

    // Primary prepares and verifies pack
    let actual_hash = blake3::hash(pack_data).to_hex().to_string();
    assert_eq!(actual_hash, pack_checksum);
    assert!(!tx.acknowledge_prepared(&primary_id).unwrap());

    // Replica 1 prepares and verifies
    let rep1_id = &placement.replica_node_ids[0];
    assert!(!tx.acknowledge_prepared(rep1_id).unwrap());

    // Replica 2 prepares and verifies
    let rep2_id = &placement.replica_node_ids[1];
    assert!(tx.acknowledge_prepared(rep2_id).unwrap());
    assert_eq!(tx.phase, TransactionPhase::Prepared);

    // Coordinator makes durable COMMIT decision
    tx.decide_commit(now + 10).unwrap();
    assert_eq!(tx.phase, TransactionPhase::CommitDecided);
    store.create_transaction(tx.clone()).unwrap();

    // Complete transaction and advance generation
    tx.mark_committed(now + 20).unwrap();
    store.update_transaction(tx.clone()).unwrap();
    store.advance_committed_generation(repo_id, 1).unwrap();

    for node_id in placement.all_nodes() {
        store
            .update_replica_state(repo_id, &node_id, node_id == primary_id, 1, true, now + 20)
            .unwrap();
    }

    let repo = store.get_repository(repo_id).unwrap().unwrap();
    assert_eq!(repo.committed_generation, 1);

    // 7. Verify Failover: Primary fails, eligible replica promoted with incremented epoch
    let replicas = store.get_replicas(repo_id).unwrap();
    let current_nodes = store.list_nodes().unwrap();
    let current_epoch = store.get_ownership_epoch(repo_id).unwrap().unwrap();

    let promo = FailoverCoordinator::evaluate_and_promote(
        repo_id,
        &primary_id,
        current_epoch,
        1,
        &replicas,
        &current_nodes,
    )
    .unwrap();

    assert_eq!(promo.repo_id, repo_id);
    assert_eq!(promo.old_primary_id, primary_id);
    assert_ne!(promo.new_primary_id, primary_id);
    assert_eq!(promo.new_ownership_epoch, 2);

    store
        .update_ownership_epoch(repo_id, promo.new_ownership_epoch, &promo.new_primary_id)
        .unwrap();

    let new_epoch = store.get_ownership_epoch(repo_id).unwrap().unwrap();
    assert_eq!(new_epoch.value(), 2);

    // 8. Fencing check: Old primary attempts write with stale epoch 1 -> strictly rejected!
    assert!(new_epoch.validate(1).is_err());
    assert!(new_epoch.validate(2).is_ok());
}
