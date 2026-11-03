use spine_api::core::types::*;
use spine_api::db::{Database, InMemoryDatabase};
use spine_api::permissions::PermissionService;
use spine_api::storage::vcs::NativeVcsService;
use spine_core::{Commit, FileMode, HashAlgorithm, ObjectId, ObjectType, TreeEntry};
use spine_format::{deserialize_commit, serialize_commit, serialize_tree};
use spine_object_store::{CompositeObjectStore, ObjectStore};
use std::fs;
use std::sync::Arc;
use tempfile::TempDir;

fn make_test_user(id: &str, username: &str) -> User {
    User {
        id: id.to_string(),
        username: username.to_string(),
        normalized_username: username.to_lowercase(),
        email: format!("{}@spine.build", username),
        normalized_email: format!("{}@spine.build", username),
        display_name: username.to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: 100,
        updated_at_nanos: 100,
        last_login_nanos: None,
    }
}

fn setup_merge_repo(tmp: &TempDir, native_repo_id: &str) -> (ObjectId, ObjectId) {
    let repo_dir = tmp.path().join("repos").join(native_repo_id);
    let spine_dir = repo_dir.join(".spine");
    fs::create_dir_all(spine_dir.join("refs").join("heads")).unwrap();
    fs::create_dir_all(spine_dir.join("objects")).unwrap();

    let store = CompositeObjectStore::new(&spine_dir, HashAlgorithm::Blake3, 3).unwrap();

    // Base commit on main
    let b_main = store.write_object(ObjectType::Blob, b"base content\n").unwrap();
    let tree_main = store.write_object(
        ObjectType::Tree,
        &serialize_tree(&[TreeEntry {
            mode: FileMode::Regular,
            name: "file.txt".into(),
            object_id: b_main,
        }]),
    ).unwrap();

    let c_main = Commit::new(
        tree_main,
        vec![],
        "Admin".into(),
        "admin@spine.build".into(),
        "Admin".into(),
        "admin@spine.build".into(),
        1000,
        "Base commit".into(),
    );
    let c_main_id = store.write_object(ObjectType::Commit, &serialize_commit(&c_main)).unwrap();
    fs::write(spine_dir.join("refs").join("heads").join("main"), format!("{}\n", c_main_id.to_hex())).unwrap();

    // Branch commit on feature
    let b_feature = store.write_object(ObjectType::Blob, b"feature new file\n").unwrap();
    let tree_feature = store.write_object(
        ObjectType::Tree,
        &serialize_tree(&[
            TreeEntry {
                mode: FileMode::Regular,
                name: "file.txt".into(),
                object_id: b_main,
            },
            TreeEntry {
                mode: FileMode::Regular,
                name: "feature.txt".into(),
                object_id: b_feature,
            },
        ]),
    ).unwrap();

    let c_feature = Commit::new(
        tree_feature,
        vec![c_main_id],
        "Contributor".into(),
        "contrib@spine.build".into(),
        "Contributor".into(),
        "contrib@spine.build".into(),
        2000,
        "Feature commit".into(),
    );
    let c_feature_id = store.write_object(ObjectType::Commit, &serialize_commit(&c_feature)).unwrap();
    fs::write(spine_dir.join("refs").join("heads").join("feature"), format!("{}\n", c_feature_id.to_hex())).unwrap();

    (c_main_id, c_feature_id)
}

#[tokio::test]
async fn test_branch_protection_and_atomic_merge() {
    let tmp = TempDir::new().unwrap();
    let native_repo_id = "test_atomic_merge";
    let (base_cid, head_cid) = setup_merge_repo(&tmp, native_repo_id);

    let db: Arc<dyn Database> = Arc::new(InMemoryDatabase::new());
    let perms = PermissionService::new(db.clone());
    let vcs = NativeVcsService::new(tmp.path());

    let admin = make_test_user("usr_admin", "admin");
    let author = make_test_user("usr_author", "author");
    let reviewer = make_test_user("usr_reviewer", "reviewer");

    db.create_user(admin.clone()).await.unwrap();
    db.create_user(author.clone()).await.unwrap();
    db.create_user(reviewer.clone()).await.unwrap();

    let repo = ForgeRepository {
        id: "repo_test_merge".into(),
        owner_user_id: admin.id.clone(),
        owner_username: admin.username.clone(),
        owner_org_id: None,
        name: "merge-project".into(),
        normalized_name: "merge-project".into(),
        description: "".into(),
        is_private: false,
        default_branch: "main".into(),
        native_repo_id: native_repo_id.into(),
        durability_policy: spine_cluster::durability::DurabilityPolicy::ThreeCopies,
        provisioning_status: ProvisioningStatus::Ready,
        created_at_nanos: 100,
        updated_at_nanos: 100,
        archived: false,
    };
    db.create_repository(repo.clone()).await.unwrap();

    // Set branch protection on main
    let rule = BranchProtectionRule {
        id: "bpr_1".into(),
        repo_id: repo.id.clone(),
        branch_pattern: "main".into(),
        require_pull_request: true,
        required_approvals: 1,
        prevent_self_approval: true,
        require_changes_addressed: true,
        dismiss_stale_reviews_on_push: true,
        restrict_merges_to_admins: false,
        created_at_nanos: 100,
        updated_at_nanos: 100,
    };
    db.set_branch_protection(rule).await.unwrap();

    let pr = PullRequest {
        id: "pr_100".into(),
        repo_id: repo.id.clone(),
        pr_number: 1,
        author_id: author.id.clone(),
        title: "Feature branch".into(),
        body: "".into(),
        base_branch: "main".into(),
        head_branch: "feature".into(),
        base_commit_id: base_cid.to_hex(),
        head_commit_id: head_cid.to_hex(),
        state: "OPEN".into(),
        is_draft: false,
        merged_by_user_id: None,
        merge_commit_id: None,
        merge_strategy: Some("MERGE_COMMIT".into()),
        closed_at_nanos: None,
        merged_at_nanos: None,
        created_at_nanos: 100,
        updated_at_nanos: 100,
    };
    db.create_pull_request(pr.clone()).await.unwrap();

    // 1. Verify merge rejected when no approvals
    assert!(perms.check_merge_allowed(&admin, &repo, "main", &pr).await.is_err());

    // 2. Verify self-approval rejected
    let self_review = PullRequestReview {
        id: "rev_self".into(),
        pr_id: pr.id.clone(),
        reviewer_id: author.id.clone(), // Self approval!
        commit_id: head_cid.to_hex(),
        state: "APPROVED".into(),
        body: "LGTM".into(),
        dismissed_by_user_id: None,
        created_at_nanos: 200,
        updated_at_nanos: 200,
    };
    db.create_pr_review(self_review).await.unwrap();
    assert!(perms.check_merge_allowed(&admin, &repo, "main", &pr).await.is_err());

    // 3. Independent approval
    let peer_review = PullRequestReview {
        id: "rev_peer".into(),
        pr_id: pr.id.clone(),
        reviewer_id: reviewer.id.clone(),
        commit_id: head_cid.to_hex(),
        state: "APPROVED".into(),
        body: "Approved by reviewer".into(),
        dismissed_by_user_id: None,
        created_at_nanos: 300,
        updated_at_nanos: 300,
    };
    db.create_pr_review(peer_review).await.unwrap();

    // Now merge allowed!
    assert!(perms.check_merge_allowed(&admin, &repo, "main", &pr).await.is_ok());

    // 4. Execute atomic 3-way merge
    let merge_res = vcs.execute_merge(
        native_repo_id,
        "main",
        "feature",
        "Admin",
        "admin@spine.build",
        "Merge branch 'feature' into main",
    ).unwrap();

    assert!(merge_res.success);
    let merge_cid_hex = merge_res.merge_commit_id.unwrap();

    // Verify main ref updated on disk
    let spine_dir = tmp.path().join("repos").join(native_repo_id).join(".spine");
    let ref_content = fs::read_to_string(spine_dir.join("refs").join("heads").join("main")).unwrap();
    assert_eq!(ref_content.trim(), merge_cid_hex);

    // Verify merge commit has 2 parents
    let store = CompositeObjectStore::new(&spine_dir, HashAlgorithm::Blake3, 3).unwrap();
    let merge_oid = ObjectId::from_hex(&merge_cid_hex).unwrap();
    let (_, payload) = store.read_object(&merge_oid).unwrap();
    let commit_obj = deserialize_commit(&payload).unwrap();
    assert_eq!(commit_obj.parents.len(), 2);
    assert_eq!(commit_obj.parents[0], base_cid);
    assert_eq!(commit_obj.parents[1], head_cid);
}
