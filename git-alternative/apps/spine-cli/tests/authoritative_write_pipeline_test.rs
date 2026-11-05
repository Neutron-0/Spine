use spine_cluster::durability::DurabilityPolicy;
use spine_cluster::placement::PlacementPlan;
use spine_cluster::store::{CoordinatorStore, FileBackedCoordinatorStore, RepositoryMeta};
use spine_cluster::write_pipeline::{
    AuthoritativeRefUpdate, AuthoritativeRepositoryWriter, BranchProtectionRule,
    LocalBackendStorageDispatcher, PrProtectionContext, WriteActor,
};
use spine_core::{Commit, FileMode, HashAlgorithm, ObjectId, ObjectType, TreeEntry};
use spine_format::{serialize_commit, serialize_tree};
use spine_net::auth::Role;
use spine_object_store::{CompositeObjectStore, ObjectStore};
use std::fs;
use std::sync::Arc;
use tempfile::TempDir;

fn setup_repo(tmp: &TempDir, repo_name: &str) -> (Arc<CompositeObjectStore>, ObjectId) {
    let repo_dir = tmp.path().join("repos").join(repo_name);
    let spine_dir = repo_dir.join(".spine");
    fs::create_dir_all(spine_dir.join("refs").join("heads")).unwrap();
    fs::create_dir_all(spine_dir.join("objects")).unwrap();

    let store = Arc::new(CompositeObjectStore::new(&spine_dir, HashAlgorithm::Blake3, 3).unwrap());

    // Create initial commit
    let b1 = store
        .write_object(ObjectType::Blob, b"hello world\n")
        .unwrap();
    let t1 = store
        .write_object(
            ObjectType::Tree,
            &serialize_tree(&[TreeEntry {
                mode: FileMode::Regular,
                name: "file.txt".into(),
                object_id: b1,
            }]),
        )
        .unwrap();

    let c1 = Commit::new(
        t1,
        vec![],
        "Alice".into(),
        "alice@spine.build".into(),
        "Alice".into(),
        "alice@spine.build".into(),
        1000,
        "Initial commit".into(),
    );
    let c1_id = store
        .write_object(ObjectType::Commit, &serialize_commit(&c1))
        .unwrap();
    fs::write(
        spine_dir.join("refs").join("heads").join("main"),
        format!("{}\n", c1_id.to_hex()),
    )
    .unwrap();

    (store, c1_id)
}

fn build_dummy_pack_bytes(
    store: &CompositeObjectStore,
    oids: &[ObjectId],
) -> (String, Vec<u8>, Vec<u8>) {
    let tmp = tempfile::tempdir().unwrap();
    let pack_id = format!("pack_{}", uuid::Uuid::new_v4().simple());
    let spk_path = tmp.path().join(format!("{}.spk", pack_id));
    let spidx_path = tmp.path().join(format!("{}.spidx", pack_id));

    spine_net::pack_transfer::create_pack_file(store, oids, &spk_path).unwrap();

    let mut pack_reader = spine_object_store::pack::PackReader::open(&spk_path).unwrap();
    let entries = pack_reader.scan_index_entries().unwrap();
    spine_object_store::pack_index::PackIndexWriter::write_index(
        &spidx_path,
        store.algorithm(),
        entries,
    )
    .unwrap();

    let pack_bytes = fs::read(&spk_path).unwrap();
    let index_bytes = fs::read(&spidx_path).unwrap();
    (pack_id, pack_bytes, index_bytes)
}

#[tokio::test]
async fn test_authoritative_write_single_copy_durability() {
    let tmp = TempDir::new().unwrap();
    let repo_id = "test_single_copy";
    let (store, base_c1) = setup_repo(&tmp, repo_id);
    let spine_dir = tmp.path().join("repos").join(repo_id).join(".spine");

    let coord_store = Arc::new(FileBackedCoordinatorStore::new_in_memory());
    let meta = RepositoryMeta {
        repo_id: repo_id.to_string(),
        name: repo_id.to_string(),
        owner: "alice".to_string(),
        durability_policy: DurabilityPolicy::SingleCopy,
        committed_generation: 0,
        current_ownership_epoch: 1,
        storage_quota_bytes: 10 * 1024 * 1024 * 1024,
        state: "HEALTHY".into(),
        created_at_nanos: 1000,
        updated_at_nanos: 1000,
    };
    coord_store.create_repository(meta).unwrap();

    let plan = PlacementPlan {
        repo_id: repo_id.to_string(),
        primary_node_id: "node_1".to_string(),
        replica_node_ids: vec![],
        required_copies: 1,
        placement_generation: 1,
    };
    coord_store.set_placement(plan).unwrap();

    let dispatcher = Arc::new(LocalBackendStorageDispatcher::new());
    let node_dir = tmp.path().join("node_1");
    fs::create_dir_all(&node_dir).unwrap();
    dispatcher.register_node("node_1".to_string(), node_dir);
    let writer = AuthoritativeRepositoryWriter::new(coord_store.clone(), dispatcher, vec![]);

    // Create a new commit to push
    let b2 = store.write_object(ObjectType::Blob, b"new line\n").unwrap();
    let t2 = store
        .write_object(
            ObjectType::Tree,
            &serialize_tree(&[TreeEntry {
                mode: FileMode::Regular,
                name: "file.txt".into(),
                object_id: b2,
            }]),
        )
        .unwrap();
    let c2 = Commit::new(
        t2,
        vec![base_c1],
        "Alice".into(),
        "alice@spine.build".into(),
        "Alice".into(),
        "alice@spine.build".into(),
        2000,
        "Update file".into(),
    );
    let c2_id = store
        .write_object(ObjectType::Commit, &serialize_commit(&c2))
        .unwrap();

    let (pack_id, pack_bytes, index_bytes) = build_dummy_pack_bytes(&store, &[c2_id, t2, b2]);

    let actor = WriteActor {
        actor_id: "usr_alice".to_string(),
        username: "alice".to_string(),
        role: Role::Contributor,
    };

    let update = AuthoritativeRefUpdate {
        ref_name: "refs/heads/main".to_string(),
        expected_old_oid: Some(base_c1),
        new_oid: c2_id,
        lease_oid: None,
        force: false,
    };

    let result = writer
        .execute_write(
            repo_id,
            &actor,
            &[update],
            &pack_id,
            &pack_bytes,
            &index_bytes,
            &spine_dir,
            &store,
            false,
            None,
        )
        .await
        .expect("Single-copy authoritative write should succeed");

    assert_eq!(result.committed_generation, 1);
    assert_eq!(result.durability_policy, DurabilityPolicy::SingleCopy);

    // Verify main ref is updated on disk
    let current_ref =
        fs::read_to_string(spine_dir.join("refs").join("heads").join("main")).unwrap();
    assert_eq!(current_ref.trim(), c2_id.to_hex());
}

#[tokio::test]
async fn test_authoritative_write_cas_failure() {
    let tmp = TempDir::new().unwrap();
    let repo_id = "test_cas_mismatch";
    let (store, base_c1) = setup_repo(&tmp, repo_id);
    let spine_dir = tmp.path().join("repos").join(repo_id).join(".spine");

    let coord_store = Arc::new(FileBackedCoordinatorStore::new_in_memory());
    let meta = RepositoryMeta {
        repo_id: repo_id.to_string(),
        name: repo_id.to_string(),
        owner: "alice".to_string(),
        durability_policy: DurabilityPolicy::SingleCopy,
        committed_generation: 0,
        current_ownership_epoch: 1,
        storage_quota_bytes: 10 * 1024 * 1024 * 1024,
        state: "HEALTHY".into(),
        created_at_nanos: 1000,
        updated_at_nanos: 1000,
    };
    coord_store.create_repository(meta).unwrap();

    let plan = PlacementPlan {
        repo_id: repo_id.to_string(),
        primary_node_id: "node_1".to_string(),
        replica_node_ids: vec![],
        required_copies: 1,
        placement_generation: 1,
    };
    coord_store.set_placement(plan).unwrap();

    let dispatcher = Arc::new(LocalBackendStorageDispatcher::new());
    let node_dir = tmp.path().join("node_1");
    fs::create_dir_all(&node_dir).unwrap();
    dispatcher.register_node("node_1".to_string(), node_dir);
    let writer = AuthoritativeRepositoryWriter::new(coord_store, dispatcher, vec![]);

    let (pack_id, pack_bytes, index_bytes) = build_dummy_pack_bytes(&store, &[base_c1]);

    let actor = WriteActor {
        actor_id: "usr_alice".to_string(),
        username: "alice".to_string(),
        role: Role::Contributor,
    };

    // Stale expected_old_oid (mismatched)
    let stale_old =
        ObjectId::from_hex("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
            .unwrap();
    let update = AuthoritativeRefUpdate {
        ref_name: "refs/heads/main".to_string(),
        expected_old_oid: Some(stale_old),
        new_oid: base_c1,
        lease_oid: None,
        force: false,
    };

    let result = writer
        .execute_write(
            repo_id,
            &actor,
            &[update],
            &pack_id,
            &pack_bytes,
            &index_bytes,
            &spine_dir,
            &store,
            false,
            None,
        )
        .await;

    assert!(result.is_err(), "Mismatched CAS old OID must be rejected");
    let err_str = result.err().unwrap().to_string();
    assert!(
        err_str.contains("Expected old reference value does not match")
            || err_str.contains("Stale reference")
    );
}

#[tokio::test]
async fn test_authoritative_write_branch_protection_enforcement() {
    let tmp = TempDir::new().unwrap();
    let repo_id = "test_branch_protection";
    let (store, base_c1) = setup_repo(&tmp, repo_id);
    let spine_dir = tmp.path().join("repos").join(repo_id).join(".spine");

    let coord_store = Arc::new(FileBackedCoordinatorStore::new_in_memory());
    let meta = RepositoryMeta {
        repo_id: repo_id.to_string(),
        name: repo_id.to_string(),
        owner: "alice".to_string(),
        durability_policy: DurabilityPolicy::SingleCopy,
        committed_generation: 0,
        current_ownership_epoch: 1,
        storage_quota_bytes: 10 * 1024 * 1024 * 1024,
        state: "HEALTHY".into(),
        created_at_nanos: 1000,
        updated_at_nanos: 1000,
    };
    coord_store.create_repository(meta).unwrap();

    let plan = PlacementPlan {
        repo_id: repo_id.to_string(),
        primary_node_id: "node_1".to_string(),
        replica_node_ids: vec![],
        required_copies: 1,
        placement_generation: 1,
    };
    coord_store.set_placement(plan).unwrap();

    let dispatcher = Arc::new(LocalBackendStorageDispatcher::new());
    let node_dir = tmp.path().join("node_1");
    fs::create_dir_all(&node_dir).unwrap();
    dispatcher.register_node("node_1".to_string(), node_dir);

    // Add branch protection rule for main branch: require PR and 2 approvals
    let rule = BranchProtectionRule {
        branch_pattern: "main".to_string(),
        require_pull_request: true,
        required_approvals: 2,
        prevent_self_approval: true,
        require_changes_addressed: true,
        allow_force_push: false,
    };
    let writer = AuthoritativeRepositoryWriter::new(coord_store, dispatcher, vec![rule]);

    let (pack_id, pack_bytes, index_bytes) = build_dummy_pack_bytes(&store, &[base_c1]);

    let actor = WriteActor {
        actor_id: "usr_alice".to_string(),
        username: "alice".to_string(),
        role: Role::Contributor,
    };

    let update = AuthoritativeRefUpdate {
        ref_name: "refs/heads/main".to_string(),
        expected_old_oid: Some(base_c1),
        new_oid: base_c1,
        lease_oid: None,
        force: false,
    };

    // Direct push should fail because PR is required
    let direct_push_result = writer
        .execute_write(
            repo_id,
            &actor,
            std::slice::from_ref(&update),
            &pack_id,
            &pack_bytes,
            &index_bytes,
            &spine_dir,
            &store,
            false,
            None,
        )
        .await;

    assert!(
        direct_push_result.is_err(),
        "Direct push must be rejected on protected branch"
    );

    // PR merge with only 1 approval should fail when 2 are required
    let pr_insufficient = PrProtectionContext {
        pr_number: 10,
        author_id: "usr_bob".to_string(),
        approved_count: 1,
        has_active_change_requests: false,
        has_unresolved_comments: false,
    };

    let pr_fail_result = writer
        .execute_write(
            repo_id,
            &actor,
            std::slice::from_ref(&update),
            &pack_id,
            &pack_bytes,
            &index_bytes,
            &spine_dir,
            &store,
            true, // is_pr_merge
            Some(&pr_insufficient),
        )
        .await;

    assert!(
        pr_fail_result.is_err(),
        "PR merge with insufficient approvals must be rejected"
    );

    // PR merge with 2 approvals should succeed
    let pr_sufficient = PrProtectionContext {
        pr_number: 10,
        author_id: "usr_bob".to_string(),
        approved_count: 2,
        has_active_change_requests: false,
        has_unresolved_comments: false,
    };

    let pr_ok_result = writer
        .execute_write(
            repo_id,
            &actor,
            &[update],
            &pack_id,
            &pack_bytes,
            &index_bytes,
            &spine_dir,
            &store,
            true, // is_pr_merge
            Some(&pr_sufficient),
        )
        .await;

    assert!(
        pr_ok_result.is_ok(),
        "PR merge with required approvals must succeed"
    );
}
