use spine_api::core::types::*;
use spine_api::db::{Database, InMemoryDatabase};
use spine_api::permissions::PermissionService;
use std::sync::Arc;

fn make_test_user(id: &str, username: &str, email: &str) -> User {
    User {
        id: id.to_string(),
        username: username.to_string(),
        normalized_username: username.to_lowercase(),
        email: email.to_string(),
        normalized_email: email.to_lowercase(),
        display_name: username.to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: 100,
        updated_at_nanos: 100,
        last_login_nanos: None,
    }
}

#[tokio::test]
async fn test_organization_and_team_lifecycle() {
    let db: Arc<dyn Database> = Arc::new(InMemoryDatabase::new());
    let perms = PermissionService::new(db.clone());

    let owner = make_test_user("usr_owner", "orgowner", "owner@spine.build");
    let member = make_test_user("usr_member", "teammember", "member@spine.build");
    let outsider = make_test_user("usr_outsider", "outsider", "outsider@spine.build");

    db.create_user(owner.clone()).await.unwrap();
    db.create_user(member.clone()).await.unwrap();
    db.create_user(outsider.clone()).await.unwrap();

    // 1. Create Organization
    let org = Organization {
        id: "org_acme".into(),
        name: "Acme Corp".into(),
        normalized_name: "acme".into(),
        display_name: "Acme Corporation".into(),
        description: "Building road runner traps".into(),
        avatar_url: None,
        owner_user_id: owner.id.clone(),
        is_private: true,
        created_at_nanos: 1000,
        updated_at_nanos: 1000,
    };
    db.create_organization(org.clone()).await.unwrap();

    // Owner membership
    let owner_mem = OrganizationMembership {
        id: "mem_owner".into(),
        org_id: org.id.clone(),
        user_id: owner.id.clone(),
        role: OrganizationRole::Owner,
        created_at_nanos: 1000,
    };
    db.add_organization_member(owner_mem).await.unwrap();

    // 2. Member invitation & acceptance
    let inv = OrganizationInvitation {
        id: "inv_1".into(),
        org_id: org.id.clone(),
        inviter_user_id: owner.id.clone(),
        invitee_email: member.email.clone(),
        role: OrganizationRole::Member,
        token_hash: "token_hash_123".into(),
        status: "PENDING".into(),
        expires_at_nanos: 2000,
        created_at_nanos: 1000,
        responded_at_nanos: None,
    };
    db.create_organization_invitation(inv.clone()).await.unwrap();

    // Lookup invitation by token
    let fetched_inv = db.get_organization_invitation_by_token("token_hash_123").await.unwrap();
    assert!(fetched_inv.is_some());
    assert_eq!(fetched_inv.unwrap().invitee_email, member.email);

    // Accept invitation
    db.update_organization_invitation_status(&inv.id, "ACCEPTED", 1100).await.unwrap();
    let member_mem = OrganizationMembership {
        id: "mem_member".into(),
        org_id: org.id.clone(),
        user_id: member.id.clone(),
        role: OrganizationRole::Member,
        created_at_nanos: 1100,
    };
    db.add_organization_member(member_mem).await.unwrap();

    // 3. Permission checks on Org
    assert!(perms.check_org_admin(&owner, &org.id).await.is_ok());
    assert!(perms.check_org_admin(&member, &org.id).await.is_err());
    assert!(perms.check_org_read_access(Some(&member), &org).await.is_ok());
    // Outsider cannot access private org (404)
    assert!(perms.check_org_read_access(Some(&outsider), &org).await.is_err());

    // 4. Teams & Team Repository Permissions
    let team = Team {
        id: "team_infra".into(),
        org_id: org.id.clone(),
        name: "Infrastructure".into(),
        normalized_slug: "infra".into(),
        description: "Cloud and bare-metal".into(),
        created_at_nanos: 1200,
        updated_at_nanos: 1200,
    };
    db.create_team(team.clone()).await.unwrap();

    // Add member to team
    db.add_team_member(TeamMembership {
        id: "tm_1".into(),
        team_id: team.id.clone(),
        user_id: member.id.clone(),
        is_maintainer: true,
        created_at_nanos: 1200,
    }).await.unwrap();

    let team_members = db.list_team_members(&team.id).await.unwrap();
    assert_eq!(team_members.len(), 1);
    assert_eq!(team_members[0].user_id, member.id);

    // Create an organization repository
    let org_repo = ForgeRepository {
        id: "repo_org_core".into(),
        owner_user_id: owner.id.clone(),
        owner_username: org.name.clone(),
        owner_org_id: Some(org.id.clone()),
        name: "core-engine".into(),
        normalized_name: "core-engine".into(),
        description: "Core Engine".into(),
        is_private: true,
        default_branch: "main".into(),
        native_repo_id: "acme_core-engine".into(),
        durability_policy: spine_cluster::durability::DurabilityPolicy::ThreeCopies,
        provisioning_status: ProvisioningStatus::Ready,
        created_at_nanos: 1300,
        updated_at_nanos: 1300,
        archived: false,
    };
    db.create_repository(org_repo.clone()).await.unwrap();

    // Grant team WRITE permission on org_repo
    db.set_team_repository_permission(TeamRepositoryPermission {
        id: "trp_1".into(),
        team_id: team.id.clone(),
        repo_id: org_repo.id.clone(),
        permission_level: TeamPermissionLevel::Write,
        created_at_nanos: 1350,
    }).await.unwrap();

    // Check member write access to org repo (inherited via team!)
    assert!(perms.check_write_access(Some(&member), &org_repo).await.is_ok());
    // Outsider has no access
    assert!(perms.check_read_access(Some(&outsider), &org_repo).await.is_err());
}
