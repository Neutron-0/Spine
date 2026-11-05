use spine_api::{
    core::types::*,
    db::{Database, PgDatabase},
};
use spine_cluster::durability::DurabilityPolicy;
use std::sync::Arc;
use uuid::Uuid;

fn get_db_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgres://spine_app:spine_secure_dev_password@172.18.77.29:5432/spine_forge?sslmode=disable"
                .to_string()
        })
}

#[tokio::test]
async fn test_postgres_live_migrations_and_concurrency() {
    let url = get_db_url();
    let db = PgDatabase::connect_and_migrate(&url)
        .await
        .expect("connect_and_migrate failed on live PostgreSQL");

    // 1. Verify repeated migration call is idempotent
    db.migrate().await.expect("idempotent migration failed");

    // 2. Test concurrent migration attempts from multiple workers (advisory lock serialized)
    let pg1 = PgDatabase::connect(&url).await.expect("pg1 connect");
    let pg2 = PgDatabase::connect(&url).await.expect("pg2 connect");
    let pg3 = PgDatabase::connect(&url).await.expect("pg3 connect");
    let pg4 = PgDatabase::connect(&url).await.expect("pg4 connect");
    let pg5 = PgDatabase::connect(&url).await.expect("pg5 connect");

    let (r1, r2, r3, r4, r5) = tokio::join!(
        pg1.migrate(),
        pg2.migrate(),
        pg3.migrate(),
        pg4.migrate(),
        pg5.migrate(),
    );

    r1.expect("concurrent migration 1 failed");
    r2.expect("concurrent migration 2 failed");
    r3.expect("concurrent migration 3 failed");
    r4.expect("concurrent migration 4 failed");
    r5.expect("concurrent migration 5 failed");
}

#[tokio::test]
async fn test_postgres_live_identity_and_auth_lifecycle() {
    let url = get_db_url();
    let db: Arc<dyn Database> = Arc::new(
        PgDatabase::connect_and_migrate(&url)
            .await
            .expect("connect failed"),
    );

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let uid = Uuid::new_v4().to_string();
    let username = format!("user_{}", &uid[..8]);
    let email = format!("user_{}@example.com", &uid[..8]);

    // Create User
    let user = User {
        id: uid.clone(),
        username: username.clone(),
        normalized_username: username.to_lowercase(),
        email: email.clone(),
        normalized_email: email.to_lowercase(),
        display_name: format!("Display {}", username),
        password_hash: "argon2id_mock_hash_123456789".to_string(),
        account_status: "active".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };

    let created = db.create_user(user.clone()).await.expect("create user");
    assert_eq!(created.id, uid);

    // Fetch user by ID, username, email
    let by_id = db.get_user_by_id(&uid).await.expect("get by id").unwrap();
    assert_eq!(by_id.username, username);

    let by_user = db
        .get_user_by_username(&username)
        .await
        .expect("get by username")
        .unwrap();
    assert_eq!(by_user.id, uid);

    let by_email = db
        .get_user_by_email(&email)
        .await
        .expect("get by email")
        .unwrap();
    assert_eq!(by_email.id, uid);

    // Update password and last login
    db.update_user_password(&uid, "argon2id_new_hash_987654321", now + 1000)
        .await
        .expect("update password");
    db.update_user_last_login(&uid, now + 2000)
        .await
        .expect("update login");

    let updated = db.get_user_by_id(&uid).await.expect("refetch").unwrap();
    assert_eq!(updated.password_hash, "argon2id_new_hash_987654321");
    assert_eq!(updated.last_login_nanos, Some(now + 2000));

    // User Sessions
    let session_id = Uuid::new_v4().to_string();
    let token_hash = format!("sess_hash_{}", &session_id[..8]);
    let session = UserSession {
        id: session_id.clone(),
        user_id: uid.clone(),
        session_token_hash: token_hash.clone(),
        ip_address: Some("127.0.0.1".to_string()),
        user_agent: Some("PostgresTestClient/1.0".to_string()),
        created_at_nanos: now,
        expires_at_nanos: now + 86400 * 1_000_000_000,
        revoked: false,
    };

    db.create_session(session).await.expect("create session");

    let fetched_sess = db
        .get_session_by_hash(&token_hash)
        .await
        .expect("get session")
        .unwrap();
    assert_eq!(fetched_sess.id, session_id);
    assert!(!fetched_sess.revoked);

    let sess_list = db.list_user_sessions(&uid).await.expect("list sessions");
    assert_eq!(sess_list.len(), 1);

    db.revoke_session(&session_id)
        .await
        .expect("revoke session");
    let after_revoke = db
        .get_session_by_hash(&token_hash)
        .await
        .expect("get session")
        .unwrap();
    assert!(after_revoke.revoked);

    // Personal Access Tokens
    let pat_id = Uuid::new_v4().to_string();
    let pat_hash = format!("pat_hash_{}", &pat_id[..8]);
    let pat = PersonalAccessToken {
        id: pat_id.clone(),
        user_id: uid.clone(),
        name: "CI Deployment Key".to_string(),
        token_hash: pat_hash.clone(),
        scopes: vec!["repo".to_string(), "read:packages".to_string()],
        expires_at_nanos: now + 86400 * 30 * 1_000_000_000,
        created_at_nanos: now,
        last_used_at_nanos: None,
        revoked: false,
    };
    db.create_pat(pat).await.expect("create pat");

    let fetched_pat = db
        .get_pat_by_hash(&pat_hash)
        .await
        .expect("get pat")
        .unwrap();
    assert_eq!(fetched_pat.name, "CI Deployment Key");

    db.update_pat_last_used(&pat_id, now + 5000)
        .await
        .expect("update pat last used");
    let pats = db.list_user_pats(&uid).await.expect("list pats");
    assert_eq!(pats.len(), 1);

    db.revoke_pat(&pat_id, &uid).await.expect("revoke pat");
    let pats_after = db.list_user_pats(&uid).await.expect("list pats after");
    assert_eq!(pats_after.len(), 0);

    // Email Tokens
    let email_token_hash = format!("email_tok_{}", &uid[..8]);
    db.create_email_token(&email_token_hash, &uid, now + 3600 * 1_000_000_000)
        .await
        .expect("create email token");

    let consumed_uid = db
        .consume_email_token(&email_token_hash, now)
        .await
        .expect("consume token");
    assert_eq!(consumed_uid, Some(uid.clone()));

    // Repeated consumption must return None (single-use)
    let re_consumed = db
        .consume_email_token(&email_token_hash, now)
        .await
        .expect("re-consume");
    assert_eq!(re_consumed, None);
}

#[tokio::test]
async fn test_postgres_live_repository_and_permissions() {
    let url = get_db_url();
    let db: Arc<dyn Database> = Arc::new(
        PgDatabase::connect_and_migrate(&url)
            .await
            .expect("connect failed"),
    );

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let owner_id = Uuid::new_v4().to_string();
    let owner_name = format!("owner_{}", &owner_id[..8]);

    // Create owner
    let owner = User {
        id: owner_id.clone(),
        username: owner_name.clone(),
        normalized_username: owner_name.to_lowercase(),
        email: format!("{}@example.com", owner_name),
        normalized_email: format!("{}@example.com", owner_name),
        display_name: owner_name.clone(),
        password_hash: "hash".to_string(),
        account_status: "active".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(owner).await.expect("create owner");

    // Create Repository
    let repo_id = Uuid::new_v4().to_string();
    let repo_name = format!("repo-{}", &repo_id[..8]);
    let repo = ForgeRepository {
        id: repo_id.clone(),
        owner_user_id: owner_id.clone(),
        owner_username: owner_name.clone(),
        owner_org_id: None,
        name: repo_name.clone(),
        normalized_name: repo_name.to_lowercase(),
        description: "Production PostgreSQL test repo".to_string(),
        is_private: false,
        default_branch: "main".to_string(),
        native_repo_id: repo_id.clone(),
        durability_policy: DurabilityPolicy::ThreeCopies,
        provisioning_status: ProvisioningStatus::Ready,
        created_at_nanos: now,
        updated_at_nanos: now,
        archived: false,
    };

    let created_repo = db.create_repository(repo).await.expect("create repo");
    assert_eq!(created_repo.id, repo_id);

    // Fetch by ID and Owner/Name
    let by_id = db
        .get_repository_by_id(&repo_id)
        .await
        .expect("get repo by id")
        .unwrap();
    assert_eq!(by_id.name, repo_name);

    let by_owner_name = db
        .get_repository_by_owner_name(&owner_name, &repo_name)
        .await
        .expect("get repo by owner name")
        .unwrap();
    assert_eq!(by_owner_name.id, repo_id);

    // Branch Protection Rules
    let rule = BranchProtectionRule {
        id: Uuid::new_v4().to_string(),
        repo_id: repo_id.clone(),
        branch_pattern: "main".to_string(),
        require_pull_request: true,
        required_approvals: 2,
        prevent_self_approval: true,
        require_changes_addressed: true,
        dismiss_stale_reviews_on_push: true,
        restrict_merges_to_admins: false,
        required_status_checks: vec!["ci/test".to_string(), "security/audit".to_string()],
        created_at_nanos: now,
        updated_at_nanos: now,
    };

    db.set_branch_protection(rule.clone())
        .await
        .expect("set branch rule");
    let rules = db
        .list_branch_protection_rules(&repo_id)
        .await
        .expect("list rules");
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].branch_pattern, "main");

    // SSH Deploy Keys
    let deploy_key_id = Uuid::new_v4().to_string();
    let fp = format!("SHA256:deploykey_{}", Uuid::new_v4().simple());
    let deploy_key = DeployKey {
        id: deploy_key_id.clone(),
        repository_id: repo_id.clone(),
        title: "Deployment Bot".to_string(),
        fingerprint: fp.clone(),
        public_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGenTestKey123".to_string(),
        key_type: "ed25519".to_string(),
        write_enabled: false,
        created_at_nanos: now,
        last_used_at_nanos: None,
        revoked: false,
    };

    db.create_deploy_key(deploy_key)
        .await
        .expect("create deploy key");
    let fetched_dk = db
        .get_deploy_key_by_fingerprint(&fp)
        .await
        .expect("get deploy key")
        .unwrap();
    assert_eq!(fetched_dk.title, "Deployment Bot");
    assert!(!fetched_dk.write_enabled);

    let keys = db
        .list_deploy_keys(&repo_id)
        .await
        .expect("list deploy keys");
    assert_eq!(keys.len(), 1);

    db.revoke_deploy_key(&deploy_key_id, &repo_id)
        .await
        .expect("revoke deploy key");

    // LFS File Locks
    let lock_id = Uuid::new_v4().to_string();
    let lock = LfsLock {
        id: lock_id.clone(),
        repository_id: repo_id.clone(),
        path: "assets/model.bin".to_string(),
        owner_id: owner_id.clone(),
        owner_username: owner_name.clone(),
        locked_at_nanos: now,
    };

    db.create_lfs_lock(lock).await.expect("create lfs lock");
    let (locks, _) = db
        .list_lfs_locks(&repo_id, None, None, None)
        .await
        .expect("list lfs locks");
    assert_eq!(locks.len(), 1);
    assert_eq!(locks[0].path, "assets/model.bin");

    db.delete_lfs_lock(&lock_id).await.expect("delete lfs lock");
    let (locks_after, _) = db
        .list_lfs_locks(&repo_id, None, None, None)
        .await
        .expect("list lfs locks after");
    assert_eq!(locks_after.len(), 0);
}

#[tokio::test]
async fn test_postgres_live_persistence_across_connections() {
    let url = get_db_url();
    // Ensure migrations are applied
    PgDatabase::connect_and_migrate(&url)
        .await
        .expect("initial connect and migrate");

    let uid = Uuid::new_v4().to_string();
    let username = format!("persist_{}", &uid[..8]);
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

    // 1. Connection 1 creates data
    {
        let db1 = PgDatabase::connect(&url).await.expect("connect 1");
        let user = User {
            id: uid.clone(),
            username: username.clone(),
            normalized_username: username.to_lowercase(),
            email: format!("{}@persist.com", username),
            normalized_email: format!("{}@persist.com", username),
            display_name: "Persistent User".to_string(),
            password_hash: "hash".to_string(),
            account_status: "active".to_string(),
            avatar_url: None,
            created_at_nanos: now,
            updated_at_nanos: now,
            last_login_nanos: None,
        };
        db1.create_user(user).await.expect("create user in conn 1");
    } // db1 dropped, pool disconnected

    // 2. Connection 2 reads data
    {
        let db2 = PgDatabase::connect(&url).await.expect("connect 2");
        let fetched = db2
            .get_user_by_id(&uid)
            .await
            .expect("fetch user in conn 2")
            .unwrap();
        assert_eq!(fetched.username, username);
        assert_eq!(fetched.display_name, "Persistent User");
    }
}
