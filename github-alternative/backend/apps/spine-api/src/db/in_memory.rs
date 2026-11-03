use super::traits::Database;
use crate::core::error::ForgeError;
use crate::core::types::*;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Default)]
struct MemoryState {
    users: HashMap<String, User>,
    user_by_username: HashMap<String, String>, // normalized_username -> id
    user_by_email: HashMap<String, String>,    // normalized_email -> id

    sessions: HashMap<String, UserSession>,
    session_by_hash: HashMap<String, String>, // hash -> session_id

    email_tokens: HashMap<String, (String, i64, Option<i64>)>, // hash -> (user_id, expires, used_at)
    reset_tokens: HashMap<String, (String, i64, Option<i64>)>, // hash -> (user_id, expires, used_at)

    pats: HashMap<String, PersonalAccessToken>,
    pat_by_hash: HashMap<String, String>, // hash -> pat_id

    repositories: HashMap<String, ForgeRepository>,
    repo_by_owner_name: HashMap<String, String>, // "owner/normalized_name" -> repo_id

    permissions: HashMap<String, RepositoryPermission>,
    perm_by_repo_user: HashMap<(String, String), String>, // (repo_id, user_id) -> perm_id

    provisioning_jobs: HashMap<String, ProvisioningJob>,

    // Organizations (Batch 09)
    organizations: HashMap<String, Organization>,
    org_by_slug: HashMap<String, String>, // normalized_name -> org_id
    org_memberships: HashMap<(String, String), OrganizationMembership>, // (org_id, user_id) -> membership
    org_invitations: HashMap<String, OrganizationInvitation>, // id -> invitation
    org_inv_by_token: HashMap<String, String>, // token_hash -> id

    // Teams (Batch 09)
    teams: HashMap<String, Team>,
    team_by_slug: HashMap<(String, String), String>, // (org_id, normalized_slug) -> team_id
    team_memberships: HashMap<(String, String), TeamMembership>, // (team_id, user_id) -> membership
    team_repo_permissions: HashMap<(String, String), TeamRepositoryPermission>, // (team_id, repo_id) -> perm

    // Issues, Labels & Milestones (Batch 09)
    milestones: HashMap<String, Milestone>,
    labels: HashMap<String, RepositoryLabel>,
    issues: HashMap<String, Issue>,
    issue_by_repo_num: HashMap<(String, i64), String>, // (repo_id, issue_num) -> issue_id
    next_issue_number: HashMap<String, i64>,
    issue_assignees: HashMap<(String, String), ()>, // (issue_id, user_id)
    issue_labels: HashMap<(String, String), ()>,    // (issue_id, label_id)
    issue_comments: HashMap<String, IssueComment>,

    // Pull Requests & Reviews (Batch 09)
    pull_requests: HashMap<String, PullRequest>,
    pr_by_repo_num: HashMap<(String, i64), String>, // (repo_id, pr_num) -> pr_id
    next_pr_number: HashMap<String, i64>,
    pr_reviewers: HashMap<(String, String), String>, // (pr_id, reviewer_id) -> requested_by
    pr_reviews: HashMap<String, PullRequestReview>,
    pr_review_comments: HashMap<String, PullRequestReviewComment>,

    // Branch Protection (Batch 09)
    branch_protection_rules: HashMap<String, BranchProtectionRule>,
    branch_prot_by_repo_pattern: HashMap<(String, String), String>, // (repo_id, pattern) -> rule_id

    // Notifications & Activity (Batch 09)
    notifications: HashMap<String, UserNotification>,
    activity_events: Vec<RepositoryActivityEvent>,
}

pub struct InMemoryDatabase {
    state: RwLock<MemoryState>,
}

impl InMemoryDatabase {
    pub fn new() -> Self {
        Self {
            state: RwLock::new(MemoryState::default()),
        }
    }
}

impl Default for InMemoryDatabase {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Database for InMemoryDatabase {
    // -------------------------------------------------------------------------
    // Users
    // -------------------------------------------------------------------------
    async fn create_user(&self, user: User) -> Result<User, ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;

        if state.user_by_username.contains_key(&user.normalized_username) {
            return Err(ForgeError::Conflict(format!(
                "Username '{}' is already taken",
                user.username
            )));
        }

        if state.user_by_email.contains_key(&user.normalized_email) {
            return Err(ForgeError::Conflict(format!(
                "Email '{}' is already registered",
                user.email
            )));
        }

        state.user_by_username.insert(user.normalized_username.clone(), user.id.clone());
        state.user_by_email.insert(user.normalized_email.clone(), user.id.clone());
        state.users.insert(user.id.clone(), user.clone());

        Ok(user)
    }

    async fn get_user_by_id(&self, id: &str) -> Result<Option<User>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state.users.get(id).cloned())
    }

    async fn get_user_by_username(&self, username: &str) -> Result<Option<User>, ForgeError> {
        let norm = username.trim().to_lowercase();
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(id) = state.user_by_username.get(&norm) {
            Ok(state.users.get(id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn get_user_by_email(&self, email: &str) -> Result<Option<User>, ForgeError> {
        let norm = email.trim().to_lowercase();
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(id) = state.user_by_email.get(&norm) {
            Ok(state.users.get(id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn update_user_last_login(&self, user_id: &str, now_nanos: i64) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(u) = state.users.get_mut(user_id) {
            u.last_login_nanos = Some(now_nanos);
            u.updated_at_nanos = now_nanos;
        }
        Ok(())
    }

    async fn update_user_password(
        &self,
        user_id: &str,
        new_hash: &str,
        now_nanos: i64,
    ) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(u) = state.users.get_mut(user_id) {
            u.password_hash = new_hash.to_string();
            u.updated_at_nanos = now_nanos;
            return Ok(());
        }
        Err(ForgeError::NotFound("User not found".into()))
    }

    // -------------------------------------------------------------------------
    // Sessions
    // -------------------------------------------------------------------------
    async fn create_session(&self, session: UserSession) -> Result<UserSession, ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        state
            .session_by_hash
            .insert(session.session_token_hash.clone(), session.id.clone());
        state.sessions.insert(session.id.clone(), session.clone());
        Ok(session)
    }

    async fn get_session_by_hash(&self, token_hash: &str) -> Result<Option<UserSession>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(id) = state.session_by_hash.get(token_hash) {
            Ok(state.sessions.get(id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn revoke_session(&self, session_id: &str) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(s) = state.sessions.get_mut(session_id) {
            s.revoked = true;
        }
        Ok(())
    }

    async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        for s in state.sessions.values_mut() {
            if s.user_id == user_id {
                s.revoked = true;
            }
        }
        Ok(())
    }

    async fn list_user_sessions(&self, user_id: &str) -> Result<Vec<UserSession>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let sessions = state
            .sessions
            .values()
            .filter(|s| s.user_id == user_id)
            .cloned()
            .collect();
        Ok(sessions)
    }

    // -------------------------------------------------------------------------
    // Verification & Reset Tokens
    // -------------------------------------------------------------------------
    async fn create_email_token(
        &self,
        token_hash: &str,
        user_id: &str,
        expires_at_nanos: i64,
    ) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.email_tokens.insert(
            token_hash.to_string(),
            (user_id.to_string(), expires_at_nanos, None),
        );
        Ok(())
    }

    async fn consume_email_token(
        &self,
        token_hash: &str,
        now_nanos: i64,
    ) -> Result<Option<String>, ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some((user_id, expires, used_at)) = state.email_tokens.get_mut(token_hash) {
            if used_at.is_some() || now_nanos > *expires {
                return Ok(None);
            }
            *used_at = Some(now_nanos);
            Ok(Some(user_id.clone()))
        } else {
            Ok(None)
        }
    }

    async fn create_reset_token(
        &self,
        token_hash: &str,
        user_id: &str,
        expires_at_nanos: i64,
    ) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.reset_tokens.insert(
            token_hash.to_string(),
            (user_id.to_string(), expires_at_nanos, None),
        );
        Ok(())
    }

    async fn consume_reset_token(
        &self,
        token_hash: &str,
        now_nanos: i64,
    ) -> Result<Option<String>, ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some((user_id, expires, used_at)) = state.reset_tokens.get_mut(token_hash) {
            if used_at.is_some() || now_nanos > *expires {
                return Ok(None);
            }
            *used_at = Some(now_nanos);
            Ok(Some(user_id.clone()))
        } else {
            Ok(None)
        }
    }

    // -------------------------------------------------------------------------
    // Personal Access Tokens
    // -------------------------------------------------------------------------
    async fn create_pat(&self, pat: PersonalAccessToken) -> Result<PersonalAccessToken, ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.pat_by_hash.insert(pat.token_hash.clone(), pat.id.clone());
        state.pats.insert(pat.id.clone(), pat.clone());
        Ok(pat)
    }

    async fn get_pat_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<PersonalAccessToken>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(id) = state.pat_by_hash.get(token_hash) {
            Ok(state.pats.get(id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn update_pat_last_used(&self, pat_id: &str, now_nanos: i64) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(p) = state.pats.get_mut(pat_id) {
            p.last_used_at_nanos = Some(now_nanos);
        }
        Ok(())
    }

    async fn list_user_pats(&self, user_id: &str) -> Result<Vec<PersonalAccessToken>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let pats = state
            .pats
            .values()
            .filter(|p| p.user_id == user_id && !p.revoked)
            .cloned()
            .collect();
        Ok(pats)
    }

    async fn revoke_pat(&self, pat_id: &str, user_id: &str) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(p) = state.pats.get_mut(pat_id) {
            if p.user_id == user_id {
                p.revoked = true;
                return Ok(());
            }
        }
        Err(ForgeError::NotFound("Personal access token not found".into()))
    }

    // -------------------------------------------------------------------------
    // Repositories
    // -------------------------------------------------------------------------
    async fn create_repository(&self, repo: ForgeRepository) -> Result<ForgeRepository, ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = format!("{}/{}", repo.owner_username.to_lowercase(), repo.normalized_name);
        if state.repo_by_owner_name.contains_key(&key) {
            return Err(ForgeError::Conflict(format!(
                "Repository '{}/{}' already exists",
                repo.owner_username, repo.name
            )));
        }

        state.repo_by_owner_name.insert(key, repo.id.clone());
        state.repositories.insert(repo.id.clone(), repo.clone());
        Ok(repo)
    }

    async fn get_repository_by_id(&self, id: &str) -> Result<Option<ForgeRepository>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state.repositories.get(id).cloned())
    }

    async fn get_repository_by_owner_name(
        &self,
        owner: &str,
        name: &str,
    ) -> Result<Option<ForgeRepository>, ForgeError> {
        let key = format!("{}/{}", owner.trim().to_lowercase(), name.trim().to_lowercase());
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(id) = state.repo_by_owner_name.get(&key) {
            Ok(state.repositories.get(id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn list_repositories_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<ForgeRepository>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut repos = Vec::new();
        for r in state.repositories.values() {
            if r.archived {
                continue;
            }
            if r.owner_user_id == user_id {
                repos.push(r.clone());
            } else if state.perm_by_repo_user.contains_key(&(r.id.clone(), user_id.to_string())) {
                repos.push(r.clone());
            } else if !r.is_private {
                repos.push(r.clone());
            }
        }
        Ok(repos)
    }

    async fn list_repositories_for_org(
        &self,
        org_id: &str,
    ) -> Result<Vec<ForgeRepository>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let repos = state
            .repositories
            .values()
            .filter(|r| !r.archived && r.owner_org_id.as_deref() == Some(org_id))
            .cloned()
            .collect();
        Ok(repos)
    }

    async fn list_public_repositories(&self) -> Result<Vec<ForgeRepository>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let repos = state
            .repositories
            .values()
            .filter(|r| !r.archived && !r.is_private)
            .cloned()
            .collect();
        Ok(repos)
    }

    async fn update_repository_status(
        &self,
        repo_id: &str,
        status: ProvisioningStatus,
        now_nanos: i64,
    ) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(r) = state.repositories.get_mut(repo_id) {
            r.provisioning_status = status;
            r.updated_at_nanos = now_nanos;
        }
        Ok(())
    }

    async fn update_repository_owner_org(
        &self,
        repo_id: &str,
        org_id: &str,
        org_name: &str,
    ) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let (old_key, new_key, r_id) = if let Some(r) = state.repositories.get(repo_id) {
            let old_key = format!("{}/{}", r.owner_username.to_lowercase(), r.normalized_name);
            let new_key = format!("{}/{}", org_name.to_lowercase(), r.normalized_name);
            (old_key, new_key, r.id.clone())
        } else {
            return Err(ForgeError::NotFound("Repository not found".into()));
        };
        state.repo_by_owner_name.remove(&old_key);
        state.repo_by_owner_name.insert(new_key, r_id);
        if let Some(r) = state.repositories.get_mut(repo_id) {
            r.owner_org_id = Some(org_id.to_string());
            r.owner_username = org_name.to_string();
        }
        Ok(())
    }

    async fn delete_repository(&self, repo_id: &str) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(r) = state.repositories.get_mut(repo_id) {
            r.archived = true;
            r.provisioning_status = ProvisioningStatus::Tombstoned;
            let key = format!("{}/{}", r.owner_username.to_lowercase(), r.normalized_name);
            state.repo_by_owner_name.remove(&key);
        }
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Permissions (Direct Collaborator)
    // -------------------------------------------------------------------------
    async fn create_permission(
        &self,
        perm: RepositoryPermission,
    ) -> Result<RepositoryPermission, ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (perm.repo_id.clone(), perm.user_id.clone());
        state.perm_by_repo_user.insert(key, perm.id.clone());
        state.permissions.insert(perm.id.clone(), perm.clone());
        Ok(perm)
    }

    async fn get_permission(
        &self,
        repo_id: &str,
        user_id: &str,
    ) -> Result<Option<PermissionRole>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (repo_id.to_string(), user_id.to_string());
        if let Some(id) = state.perm_by_repo_user.get(&key) {
            Ok(state.permissions.get(id).map(|p| p.role))
        } else {
            Ok(None)
        }
    }

    async fn list_permissions_for_repo(
        &self,
        repo_id: &str,
    ) -> Result<Vec<RepositoryPermission>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let perms = state
            .permissions
            .values()
            .filter(|p| p.repo_id == repo_id)
            .cloned()
            .collect();
        Ok(perms)
    }

    async fn delete_permission(&self, repo_id: &str, user_id: &str) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (repo_id.to_string(), user_id.to_string());
        if let Some(id) = state.perm_by_repo_user.remove(&key) {
            state.permissions.remove(&id);
        }
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Provisioning Jobs
    // -------------------------------------------------------------------------
    async fn create_provisioning_job(
        &self,
        job: ProvisioningJob,
    ) -> Result<ProvisioningJob, ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.provisioning_jobs.insert(job.id.clone(), job.clone());
        Ok(job)
    }

    async fn get_next_pending_job(&self) -> Result<Option<ProvisioningJob>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let job = state
            .provisioning_jobs
            .values()
            .find(|j| j.status == ProvisioningStatus::Pending)
            .cloned();
        Ok(job)
    }

    async fn update_job_status(
        &self,
        job_id: &str,
        status: ProvisioningStatus,
        last_error: Option<String>,
        now_nanos: i64,
    ) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(j) = state.provisioning_jobs.get_mut(job_id) {
            j.status = status;
            j.last_error = last_error;
            j.updated_at_nanos = now_nanos;
        }
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Organizations & Membership (Batch 09)
    // -------------------------------------------------------------------------
    async fn create_organization(&self, org: Organization) -> Result<Organization, ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if state.org_by_slug.contains_key(&org.normalized_name) {
            return Err(ForgeError::Conflict(format!(
                "Organization '{}' already exists",
                org.name
            )));
        }

        state.org_by_slug.insert(org.normalized_name.clone(), org.id.clone());
        state.organizations.insert(org.id.clone(), org.clone());
        Ok(org)
    }

    async fn get_organization_by_id(&self, id: &str) -> Result<Option<Organization>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state.organizations.get(id).cloned())
    }

    async fn get_organization_by_slug(&self, slug: &str) -> Result<Option<Organization>, ForgeError> {
        let norm = slug.trim().to_lowercase();
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(id) = state.org_by_slug.get(&norm) {
            Ok(state.organizations.get(id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn list_organizations_for_user(&self, user_id: &str) -> Result<Vec<Organization>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut orgs = Vec::new();
        for ((org_id, m_user_id), _) in &state.org_memberships {
            if m_user_id == user_id {
                if let Some(o) = state.organizations.get(org_id) {
                    orgs.push(o.clone());
                }
            }
        }
        Ok(orgs)
    }

    async fn update_organization(&self, org: Organization) -> Result<Organization, ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(existing) = state.organizations.get_mut(&org.id) {
            *existing = org.clone();
            return Ok(org);
        }
        Err(ForgeError::NotFound("Organization not found".into()))
    }

    async fn delete_organization(&self, org_id: &str) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(org) = state.organizations.remove(org_id) {
            state.org_by_slug.remove(&org.normalized_name);
        }
        Ok(())
    }

    async fn add_organization_member(&self, membership: OrganizationMembership) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (membership.org_id.clone(), membership.user_id.clone());
        state.org_memberships.insert(key, membership);
        Ok(())
    }

    async fn get_organization_membership(
        &self,
        org_id: &str,
        user_id: &str,
    ) -> Result<Option<OrganizationMembership>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (org_id.to_string(), user_id.to_string());
        Ok(state.org_memberships.get(&key).cloned())
    }

    async fn list_organization_members(&self, org_id: &str) -> Result<Vec<OrganizationMembership>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let members = state
            .org_memberships
            .values()
            .filter(|m| m.org_id == org_id)
            .cloned()
            .collect();
        Ok(members)
    }

    async fn remove_organization_member(&self, org_id: &str, user_id: &str) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (org_id.to_string(), user_id.to_string());
        state.org_memberships.remove(&key);
        Ok(())
    }

    async fn update_organization_member_role(
        &self,
        org_id: &str,
        user_id: &str,
        role: OrganizationRole,
    ) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (org_id.to_string(), user_id.to_string());
        if let Some(m) = state.org_memberships.get_mut(&key) {
            m.role = role;
            return Ok(());
        }
        Err(ForgeError::NotFound("Membership not found".into()))
    }

    async fn create_organization_invitation(&self, inv: OrganizationInvitation) -> Result<OrganizationInvitation, ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.org_inv_by_token.insert(inv.token_hash.clone(), inv.id.clone());
        state.org_invitations.insert(inv.id.clone(), inv.clone());
        Ok(inv)
    }

    async fn get_organization_invitation_by_token(
        &self,
        token_hash: &str,
    ) -> Result<Option<OrganizationInvitation>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(id) = state.org_inv_by_token.get(token_hash) {
            Ok(state.org_invitations.get(id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn list_organization_invitations(&self, org_id: &str) -> Result<Vec<OrganizationInvitation>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let list = state
            .org_invitations
            .values()
            .filter(|i| i.org_id == org_id)
            .cloned()
            .collect();
        Ok(list)
    }

    async fn update_organization_invitation_status(
        &self,
        id: &str,
        status: &str,
        now_nanos: i64,
    ) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(inv) = state.org_invitations.get_mut(id) {
            inv.status = status.to_string();
            inv.responded_at_nanos = Some(now_nanos);
            return Ok(());
        }
        Err(ForgeError::NotFound("Invitation not found".into()))
    }

    // -------------------------------------------------------------------------
    // Teams & Permissions (Batch 09)
    // -------------------------------------------------------------------------
    async fn create_team(&self, team: Team) -> Result<Team, ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (team.org_id.clone(), team.normalized_slug.clone());
        if state.team_by_slug.contains_key(&key) {
            return Err(ForgeError::Conflict(format!(
                "Team slug '{}' already exists in organization",
                team.normalized_slug
            )));
        }

        state.team_by_slug.insert(key, team.id.clone());
        state.teams.insert(team.id.clone(), team.clone());
        Ok(team)
    }

    async fn get_team_by_id(&self, id: &str) -> Result<Option<Team>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state.teams.get(id).cloned())
    }

    async fn get_team_by_slug(&self, org_id: &str, slug: &str) -> Result<Option<Team>, ForgeError> {
        let norm = slug.trim().to_lowercase();
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (org_id.to_string(), norm);
        if let Some(id) = state.team_by_slug.get(&key) {
            Ok(state.teams.get(id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn list_teams_for_org(&self, org_id: &str) -> Result<Vec<Team>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let teams = state
            .teams
            .values()
            .filter(|t| t.org_id == org_id)
            .cloned()
            .collect();
        Ok(teams)
    }

    async fn delete_team(&self, team_id: &str) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(team) = state.teams.remove(team_id) {
            state.team_by_slug.remove(&(team.org_id, team.normalized_slug));
        }
        Ok(())
    }

    async fn add_team_member(&self, membership: TeamMembership) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (membership.team_id.clone(), membership.user_id.clone());
        state.team_memberships.insert(key, membership);
        Ok(())
    }

    async fn remove_team_member(&self, team_id: &str, user_id: &str) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (team_id.to_string(), user_id.to_string());
        state.team_memberships.remove(&key);
        Ok(())
    }

    async fn list_team_members(&self, team_id: &str) -> Result<Vec<TeamMembership>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let members = state
            .team_memberships
            .values()
            .filter(|m| m.team_id == team_id)
            .cloned()
            .collect();
        Ok(members)
    }

    async fn set_team_repository_permission(
        &self,
        perm: TeamRepositoryPermission,
    ) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (perm.team_id.clone(), perm.repo_id.clone());
        state.team_repo_permissions.insert(key, perm);
        Ok(())
    }

    async fn get_team_repository_permission(
        &self,
        team_id: &str,
        repo_id: &str,
    ) -> Result<Option<TeamPermissionLevel>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (team_id.to_string(), repo_id.to_string());
        Ok(state.team_repo_permissions.get(&key).map(|p| p.permission_level))
    }

    async fn list_team_repository_permissions_for_repo(
        &self,
        repo_id: &str,
    ) -> Result<Vec<TeamRepositoryPermission>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let list = state
            .team_repo_permissions
            .values()
            .filter(|p| p.repo_id == repo_id)
            .cloned()
            .collect();
        Ok(list)
    }

    async fn list_teams_for_user_in_org(
        &self,
        org_id: &str,
        user_id: &str,
    ) -> Result<Vec<Team>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut user_teams = Vec::new();
        for ((team_id, m_user_id), _) in &state.team_memberships {
            if m_user_id == user_id {
                if let Some(t) = state.teams.get(team_id) {
                    if t.org_id == org_id {
                        user_teams.push(t.clone());
                    }
                }
            }
        }
        Ok(user_teams)
    }

    // -------------------------------------------------------------------------
    // Milestones & Labels (Batch 09)
    // -------------------------------------------------------------------------
    async fn create_milestone(&self, milestone: Milestone) -> Result<Milestone, ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.milestones.insert(milestone.id.clone(), milestone.clone());
        Ok(milestone)
    }

    async fn get_milestone_by_id(&self, id: &str) -> Result<Option<Milestone>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state.milestones.get(id).cloned())
    }

    async fn list_milestones_for_repo(&self, repo_id: &str) -> Result<Vec<Milestone>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let list = state
            .milestones
            .values()
            .filter(|m| m.repo_id == repo_id)
            .cloned()
            .collect();
        Ok(list)
    }

    async fn update_milestone(&self, milestone: Milestone) -> Result<Milestone, ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(m) = state.milestones.get_mut(&milestone.id) {
            *m = milestone.clone();
            return Ok(milestone);
        }
        Err(ForgeError::NotFound("Milestone not found".into()))
    }

    async fn delete_milestone(&self, id: &str) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.milestones.remove(id);
        Ok(())
    }

    async fn create_label(&self, label: RepositoryLabel) -> Result<RepositoryLabel, ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.labels.insert(label.id.clone(), label.clone());
        Ok(label)
    }

    async fn list_labels_for_repo(&self, repo_id: &str) -> Result<Vec<RepositoryLabel>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let list = state
            .labels
            .values()
            .filter(|l| l.repo_id == repo_id)
            .cloned()
            .collect();
        Ok(list)
    }

    async fn delete_label(&self, id: &str) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.labels.remove(id);
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Issues & Issue Comments (Batch 09)
    // -------------------------------------------------------------------------
    async fn create_issue(&self, mut issue: Issue) -> Result<Issue, ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let next_num = state.next_issue_number.entry(issue.repo_id.clone()).or_insert(0);
        *next_num += 1;
        issue.issue_number = *next_num;

        let key = (issue.repo_id.clone(), issue.issue_number);
        state.issue_by_repo_num.insert(key, issue.id.clone());
        state.issues.insert(issue.id.clone(), issue.clone());
        Ok(issue)
    }

    async fn get_issue_by_number(
        &self,
        repo_id: &str,
        issue_number: i64,
    ) -> Result<Option<Issue>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (repo_id.to_string(), issue_number);
        if let Some(id) = state.issue_by_repo_num.get(&key) {
            Ok(state.issues.get(id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn list_issues_for_repo(
        &self,
        repo_id: &str,
        status: Option<&str>,
    ) -> Result<Vec<Issue>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let issues = state
            .issues
            .values()
            .filter(|i| i.repo_id == repo_id && (status.is_none() || status == Some(i.status.as_str())))
            .cloned()
            .collect();
        Ok(issues)
    }

    async fn update_issue(&self, issue: Issue) -> Result<Issue, ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(i) = state.issues.get_mut(&issue.id) {
            *i = issue.clone();
            return Ok(issue);
        }
        Err(ForgeError::NotFound("Issue not found".into()))
    }

    async fn assign_issue(&self, issue_id: &str, user_id: &str) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (issue_id.to_string(), user_id.to_string());
        state.issue_assignees.insert(key, ());
        Ok(())
    }

    async fn unassign_issue(&self, issue_id: &str, user_id: &str) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (issue_id.to_string(), user_id.to_string());
        state.issue_assignees.remove(&key);
        Ok(())
    }

    async fn list_issue_assignees(&self, issue_id: &str) -> Result<Vec<String>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let assignees = state
            .issue_assignees
            .keys()
            .filter(|(i_id, _)| i_id == issue_id)
            .map(|(_, u_id)| u_id.clone())
            .collect();
        Ok(assignees)
    }

    async fn add_issue_label(&self, issue_id: &str, label_id: &str) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (issue_id.to_string(), label_id.to_string());
        state.issue_labels.insert(key, ());
        Ok(())
    }

    async fn remove_issue_label(&self, issue_id: &str, label_id: &str) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (issue_id.to_string(), label_id.to_string());
        state.issue_labels.remove(&key);
        Ok(())
    }

    async fn list_issue_labels(&self, issue_id: &str) -> Result<Vec<String>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let labels = state
            .issue_labels
            .keys()
            .filter(|(i_id, _)| i_id == issue_id)
            .map(|(_, l_id)| l_id.clone())
            .collect();
        Ok(labels)
    }

    async fn create_issue_comment(&self, comment: IssueComment) -> Result<IssueComment, ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.issue_comments.insert(comment.id.clone(), comment.clone());
        Ok(comment)
    }

    async fn list_issue_comments(&self, issue_id: &str) -> Result<Vec<IssueComment>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut comments: Vec<IssueComment> = state
            .issue_comments
            .values()
            .filter(|c| c.issue_id == issue_id)
            .cloned()
            .collect();
        comments.sort_by_key(|c| c.created_at_nanos);
        Ok(comments)
    }

    // -------------------------------------------------------------------------
    // Pull Requests, Reviews & Inline Comments (Batch 09)
    // -------------------------------------------------------------------------
    async fn create_pull_request(&self, mut pr: PullRequest) -> Result<PullRequest, ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let next_num = state.next_pr_number.entry(pr.repo_id.clone()).or_insert(0);
        *next_num += 1;
        pr.pr_number = *next_num;

        let key = (pr.repo_id.clone(), pr.pr_number);
        state.pr_by_repo_num.insert(key, pr.id.clone());
        state.pull_requests.insert(pr.id.clone(), pr.clone());
        Ok(pr)
    }

    async fn get_pull_request_by_number(
        &self,
        repo_id: &str,
        pr_number: i64,
    ) -> Result<Option<PullRequest>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (repo_id.to_string(), pr_number);
        if let Some(id) = state.pr_by_repo_num.get(&key) {
            Ok(state.pull_requests.get(id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn list_pull_requests_for_repo(
        &self,
        repo_id: &str,
        state_filter: Option<&str>,
    ) -> Result<Vec<PullRequest>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let prs = state
            .pull_requests
            .values()
            .filter(|p| p.repo_id == repo_id && (state_filter.is_none() || state_filter == Some(p.state.as_str())))
            .cloned()
            .collect();
        Ok(prs)
    }

    async fn update_pull_request(&self, pr: PullRequest) -> Result<PullRequest, ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(p) = state.pull_requests.get_mut(&pr.id) {
            *p = pr.clone();
            return Ok(pr);
        }
        Err(ForgeError::NotFound("Pull request not found".into()))
    }

    async fn request_pr_reviewer(
        &self,
        pr_id: &str,
        reviewer_id: &str,
        requested_by: &str,
    ) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (pr_id.to_string(), reviewer_id.to_string());
        state.pr_reviewers.insert(key, requested_by.to_string());
        Ok(())
    }

    async fn list_pr_requested_reviewers(&self, pr_id: &str) -> Result<Vec<String>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let reviewers = state
            .pr_reviewers
            .keys()
            .filter(|(p_id, _)| p_id == pr_id)
            .map(|(_, r_id)| r_id.clone())
            .collect();
        Ok(reviewers)
    }

    async fn create_pr_review(&self, review: PullRequestReview) -> Result<PullRequestReview, ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.pr_reviews.insert(review.id.clone(), review.clone());
        Ok(review)
    }

    async fn list_pr_reviews(&self, pr_id: &str) -> Result<Vec<PullRequestReview>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut reviews: Vec<PullRequestReview> = state
            .pr_reviews
            .values()
            .filter(|r| r.pr_id == pr_id)
            .cloned()
            .collect();
        reviews.sort_by_key(|r| r.created_at_nanos);
        Ok(reviews)
    }

    async fn dismiss_pr_review(
        &self,
        review_id: &str,
        dismissed_by: &str,
        now_nanos: i64,
    ) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(r) = state.pr_reviews.get_mut(review_id) {
            r.state = "DISMISSED".to_string();
            r.dismissed_by_user_id = Some(dismissed_by.to_string());
            r.updated_at_nanos = now_nanos;
            return Ok(());
        }
        Err(ForgeError::NotFound("Review not found".into()))
    }

    async fn create_pr_review_comment(
        &self,
        comment: PullRequestReviewComment,
    ) -> Result<PullRequestReviewComment, ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.pr_review_comments.insert(comment.id.clone(), comment.clone());
        Ok(comment)
    }

    async fn list_pr_review_comments(
        &self,
        pr_id: &str,
    ) -> Result<Vec<PullRequestReviewComment>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut comments: Vec<PullRequestReviewComment> = state
            .pr_review_comments
            .values()
            .filter(|c| c.pr_id == pr_id)
            .cloned()
            .collect();
        comments.sort_by_key(|c| c.created_at_nanos);
        Ok(comments)
    }

    async fn resolve_pr_review_comment(&self, comment_id: &str, resolved: bool) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(c) = state.pr_review_comments.get_mut(comment_id) {
            c.is_resolved = resolved;
            return Ok(());
        }
        Err(ForgeError::NotFound("Comment not found".into()))
    }

    // -------------------------------------------------------------------------
    // Branch Protection Rules (Batch 09)
    // -------------------------------------------------------------------------
    async fn set_branch_protection(
        &self,
        rule: BranchProtectionRule,
    ) -> Result<BranchProtectionRule, ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (rule.repo_id.clone(), rule.branch_pattern.clone());
        state.branch_prot_by_repo_pattern.insert(key, rule.id.clone());
        state.branch_protection_rules.insert(rule.id.clone(), rule.clone());
        Ok(rule)
    }

    async fn get_branch_protection(
        &self,
        repo_id: &str,
        branch: &str,
    ) -> Result<Option<BranchProtectionRule>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        // Check exact match or glob wildcard match
        for r in state.branch_protection_rules.values() {
            if r.repo_id == repo_id {
                if r.branch_pattern == branch || r.branch_pattern == "*" {
                    return Ok(Some(r.clone()));
                }
            }
        }
        Ok(None)
    }

    async fn list_branch_protection_rules(
        &self,
        repo_id: &str,
    ) -> Result<Vec<BranchProtectionRule>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let rules = state
            .branch_protection_rules
            .values()
            .filter(|r| r.repo_id == repo_id)
            .cloned()
            .collect();
        Ok(rules)
    }

    async fn delete_branch_protection(&self, rule_id: &str) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(r) = state.branch_protection_rules.remove(rule_id) {
            state.branch_prot_by_repo_pattern.remove(&(r.repo_id, r.branch_pattern));
        }
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Notifications & Activity Events (Batch 09)
    // -------------------------------------------------------------------------
    async fn create_notification(&self, notif: UserNotification) -> Result<UserNotification, ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.notifications.insert(notif.id.clone(), notif.clone());
        Ok(notif)
    }

    async fn list_user_notifications(
        &self,
        user_id: &str,
        unread_only: bool,
    ) -> Result<Vec<UserNotification>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut list: Vec<UserNotification> = state
            .notifications
            .values()
            .filter(|n| n.user_id == user_id && (!unread_only || !n.is_read))
            .cloned()
            .collect();
        list.sort_by_key(|n| std::cmp::Reverse(n.created_at_nanos));
        Ok(list)
    }

    async fn mark_notification_read(&self, notif_id: &str, user_id: &str) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(n) = state.notifications.get_mut(notif_id) {
            if n.user_id == user_id {
                n.is_read = true;
                return Ok(());
            }
        }
        Err(ForgeError::NotFound("Notification not found".into()))
    }

    async fn mark_all_notifications_read(&self, user_id: &str) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        for n in state.notifications.values_mut() {
            if n.user_id == user_id {
                n.is_read = true;
            }
        }
        Ok(())
    }

    async fn record_activity_event(&self, event: RepositoryActivityEvent) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.activity_events.push(event);
        Ok(())
    }

    async fn list_activity_events_for_repo(
        &self,
        repo_id: &str,
        limit: usize,
    ) -> Result<Vec<RepositoryActivityEvent>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut events: Vec<RepositoryActivityEvent> = state
            .activity_events
            .iter()
            .filter(|e| e.repo_id == repo_id)
            .cloned()
            .collect();
        events.sort_by_key(|e| std::cmp::Reverse(e.created_at_nanos));
        if events.len() > limit {
            events.truncate(limit);
        }
        Ok(events)
    }

    async fn record_audit_event(&self, _event: AuditEvent) -> Result<(), ForgeError> {
        Ok(())
    }
}
