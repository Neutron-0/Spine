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
    org_invitations: HashMap<String, OrganizationInvitation>,           // id -> invitation
    org_inv_by_token: HashMap<String, String>,                          // token_hash -> id

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

    // Artifacts, Releases & Packages (Batch 11)
    quotas: HashMap<String, ArtifactQuota>,
    upload_sessions: HashMap<String, ArtifactUploadSession>,
    manifests: HashMap<String, StoredArtifactManifest>,
    manifests_by_artifact_id: HashMap<String, StoredArtifactManifest>,
    releases: HashMap<String, Release>,
    release_by_repo_tag: HashMap<(String, String), String>,
    release_assets: HashMap<String, ReleaseAsset>,
    artifact_namespaces: HashMap<String, ArtifactNamespace>,
    artifact_ns_by_name: HashMap<String, String>,
    artifact_versions: HashMap<String, ArtifactVersion>,
    packages: HashMap<String, Package>,
    pkg_by_eco_name: HashMap<(String, String), String>,
    package_versions: HashMap<String, PackageVersion>,

    // Webhooks, Workflows & Runners (Batch 12)
    webhook_subscriptions: HashMap<String, WebhookSubscription>,
    webhook_deliveries: HashMap<String, WebhookDelivery>,
    webhook_outbox: HashMap<String, WebhookOutboxItem>,

    workflow_definitions: HashMap<String, WorkflowDefinition>,
    workflow_runs: HashMap<String, WorkflowRun>,
    workflow_jobs: HashMap<String, WorkflowJob>,
    workflow_steps: HashMap<String, WorkflowStep>,
    workflow_logs: Vec<WorkflowLogChunk>,

    runners: HashMap<String, RunnerRegistration>,
    runner_by_token_hash: HashMap<String, String>,

    workflow_secrets: HashMap<String, WorkflowSecret>,
    workflow_check_results: HashMap<(String, String, String), WorkflowCheckResult>,

    // Discussions (Batch 13)
    discussion_categories: HashMap<String, DiscussionCategory>,
    discussions: HashMap<String, Discussion>,
    discussion_by_repo_num: HashMap<(String, i32), String>,
    next_discussion_number: HashMap<String, i32>,
    discussion_comments: HashMap<String, DiscussionComment>,
    discussion_reactions: Vec<DiscussionReaction>,
    discussion_subscriptions: HashMap<(String, String), DiscussionSubscription>,

    // Topics, Languages, Profiles (Batch 13)
    repository_topics: HashMap<String, Vec<RepositoryTopic>>,
    repository_languages: HashMap<String, Vec<RepositoryLanguage>>,
    user_profiles: HashMap<String, ExtendedUserProfile>,

    // Security Center (Batch 13)
    security_scans: HashMap<String, SecurityScan>,
    secret_scan_findings: HashMap<String, SecretScanFinding>,
    dependency_inventories: HashMap<String, DependencyInventory>,
    security_advisories: HashMap<String, SecurityAdvisory>,
    dependency_vulnerabilities: HashMap<String, DependencyVulnerability>,

    // SSH Keys, Deploy Keys, and LFS Locks (Batch 18)
    user_ssh_keys: HashMap<String, UserSshKey>,
    user_ssh_by_fingerprint: HashMap<String, String>,
    deploy_keys: HashMap<String, DeployKey>,
    deploy_by_fingerprint: HashMap<String, String>,
    lfs_locks: HashMap<String, LfsLock>,
    lfs_locks_by_repo_path: HashMap<(String, String), String>,
    lfs_lock_audits: Vec<LfsLockAudit>,
    lfs_objects: HashMap<String, LfsObject>,
    lfs_objects_by_repo_oid: HashMap<(String, String), String>,
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
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if state
            .user_by_username
            .contains_key(&user.normalized_username)
        {
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

        state
            .user_by_username
            .insert(user.normalized_username.clone(), user.id.clone());
        state
            .user_by_email
            .insert(user.normalized_email.clone(), user.id.clone());
        state.users.insert(user.id.clone(), user.clone());

        Ok(user)
    }

    async fn get_user_by_id(&self, id: &str) -> Result<Option<User>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state.users.get(id).cloned())
    }

    async fn get_user_by_username(&self, username: &str) -> Result<Option<User>, ForgeError> {
        let norm = username.trim().to_lowercase();
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(id) = state.user_by_username.get(&norm) {
            Ok(state.users.get(id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn get_user_by_email(&self, email: &str) -> Result<Option<User>, ForgeError> {
        let norm = email.trim().to_lowercase();
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(id) = state.user_by_email.get(&norm) {
            Ok(state.users.get(id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn update_user_last_login(
        &self,
        user_id: &str,
        now_nanos: i64,
    ) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state
            .session_by_hash
            .insert(session.session_token_hash.clone(), session.id.clone());
        state.sessions.insert(session.id.clone(), session.clone());
        Ok(session)
    }

    async fn get_session_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<UserSession>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(id) = state.session_by_hash.get(token_hash) {
            Ok(state.sessions.get(id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn revoke_session(&self, session_id: &str) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(s) = state.sessions.get_mut(session_id) {
            s.revoked = true;
        }
        Ok(())
    }

    async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        for s in state.sessions.values_mut() {
            if s.user_id == user_id {
                s.revoked = true;
            }
        }
        Ok(())
    }

    async fn list_user_sessions(&self, user_id: &str) -> Result<Vec<UserSession>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
    async fn create_pat(
        &self,
        pat: PersonalAccessToken,
    ) -> Result<PersonalAccessToken, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state
            .pat_by_hash
            .insert(pat.token_hash.clone(), pat.id.clone());
        state.pats.insert(pat.id.clone(), pat.clone());
        Ok(pat)
    }

    async fn get_pat_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<PersonalAccessToken>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(id) = state.pat_by_hash.get(token_hash) {
            Ok(state.pats.get(id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn update_pat_last_used(&self, pat_id: &str, now_nanos: i64) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(p) = state.pats.get_mut(pat_id) {
            p.last_used_at_nanos = Some(now_nanos);
        }
        Ok(())
    }

    async fn list_user_pats(&self, user_id: &str) -> Result<Vec<PersonalAccessToken>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let pats = state
            .pats
            .values()
            .filter(|p| p.user_id == user_id && !p.revoked)
            .cloned()
            .collect();
        Ok(pats)
    }

    async fn revoke_pat(&self, pat_id: &str, user_id: &str) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(p) = state.pats.get_mut(pat_id) {
            if p.user_id == user_id {
                p.revoked = true;
                return Ok(());
            }
        }
        Err(ForgeError::NotFound(
            "Personal access token not found".into(),
        ))
    }

    // -------------------------------------------------------------------------
    // Repositories
    // -------------------------------------------------------------------------
    async fn create_repository(
        &self,
        repo: ForgeRepository,
    ) -> Result<ForgeRepository, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = format!(
            "{}/{}",
            repo.owner_username.to_lowercase(),
            repo.normalized_name
        );
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
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state.repositories.get(id).cloned())
    }

    async fn get_repository_by_owner_name(
        &self,
        owner: &str,
        name: &str,
    ) -> Result<Option<ForgeRepository>, ForgeError> {
        let key = format!(
            "{}/{}",
            owner.trim().to_lowercase(),
            name.trim().to_lowercase()
        );
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut repos = Vec::new();
        for r in state.repositories.values() {
            if r.archived {
                continue;
            }
            if r.owner_user_id == user_id
                || state
                    .perm_by_repo_user
                    .contains_key(&(r.id.clone(), user_id.to_string()))
                || !r.is_private
            {
                repos.push(r.clone());
            }
        }
        Ok(repos)
    }

    async fn list_repositories_for_org(
        &self,
        org_id: &str,
    ) -> Result<Vec<ForgeRepository>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let repos = state
            .repositories
            .values()
            .filter(|r| !r.archived && r.owner_org_id.as_deref() == Some(org_id))
            .cloned()
            .collect();
        Ok(repos)
    }

    async fn list_public_repositories(&self) -> Result<Vec<ForgeRepository>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let perms = state
            .permissions
            .values()
            .filter(|p| p.repo_id == repo_id)
            .cloned()
            .collect();
        Ok(perms)
    }

    async fn delete_permission(&self, repo_id: &str, user_id: &str) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.provisioning_jobs.insert(job.id.clone(), job.clone());
        Ok(job)
    }

    async fn get_next_pending_job(&self) -> Result<Option<ProvisioningJob>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if state.org_by_slug.contains_key(&org.normalized_name) {
            return Err(ForgeError::Conflict(format!(
                "Organization '{}' already exists",
                org.name
            )));
        }

        state
            .org_by_slug
            .insert(org.normalized_name.clone(), org.id.clone());
        state.organizations.insert(org.id.clone(), org.clone());
        Ok(org)
    }

    async fn get_organization_by_id(&self, id: &str) -> Result<Option<Organization>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state.organizations.get(id).cloned())
    }

    async fn get_organization_by_slug(
        &self,
        slug: &str,
    ) -> Result<Option<Organization>, ForgeError> {
        let norm = slug.trim().to_lowercase();
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(id) = state.org_by_slug.get(&norm) {
            Ok(state.organizations.get(id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn list_organizations_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<Organization>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut orgs = Vec::new();
        for (org_id, m_user_id) in state.org_memberships.keys() {
            if m_user_id == user_id {
                if let Some(o) = state.organizations.get(org_id) {
                    orgs.push(o.clone());
                }
            }
        }
        Ok(orgs)
    }

    async fn update_organization(&self, org: Organization) -> Result<Organization, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(existing) = state.organizations.get_mut(&org.id) {
            *existing = org.clone();
            return Ok(org);
        }
        Err(ForgeError::NotFound("Organization not found".into()))
    }

    async fn delete_organization(&self, org_id: &str) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(org) = state.organizations.remove(org_id) {
            state.org_by_slug.remove(&org.normalized_name);
        }
        Ok(())
    }

    async fn add_organization_member(
        &self,
        membership: OrganizationMembership,
    ) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (membership.org_id.clone(), membership.user_id.clone());
        state.org_memberships.insert(key, membership);
        Ok(())
    }

    async fn get_organization_membership(
        &self,
        org_id: &str,
        user_id: &str,
    ) -> Result<Option<OrganizationMembership>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (org_id.to_string(), user_id.to_string());
        Ok(state.org_memberships.get(&key).cloned())
    }

    async fn list_organization_members(
        &self,
        org_id: &str,
    ) -> Result<Vec<OrganizationMembership>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let members = state
            .org_memberships
            .values()
            .filter(|m| m.org_id == org_id)
            .cloned()
            .collect();
        Ok(members)
    }

    async fn remove_organization_member(
        &self,
        org_id: &str,
        user_id: &str,
    ) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (org_id.to_string(), user_id.to_string());
        if let Some(m) = state.org_memberships.get_mut(&key) {
            m.role = role;
            return Ok(());
        }
        Err(ForgeError::NotFound("Membership not found".into()))
    }

    async fn create_organization_invitation(
        &self,
        inv: OrganizationInvitation,
    ) -> Result<OrganizationInvitation, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state
            .org_inv_by_token
            .insert(inv.token_hash.clone(), inv.id.clone());
        state.org_invitations.insert(inv.id.clone(), inv.clone());
        Ok(inv)
    }

    async fn get_organization_invitation_by_token(
        &self,
        token_hash: &str,
    ) -> Result<Option<OrganizationInvitation>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(id) = state.org_inv_by_token.get(token_hash) {
            Ok(state.org_invitations.get(id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn list_organization_invitations(
        &self,
        org_id: &str,
    ) -> Result<Vec<OrganizationInvitation>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state.teams.get(id).cloned())
    }

    async fn get_team_by_slug(&self, org_id: &str, slug: &str) -> Result<Option<Team>, ForgeError> {
        let norm = slug.trim().to_lowercase();
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (org_id.to_string(), norm);
        if let Some(id) = state.team_by_slug.get(&key) {
            Ok(state.teams.get(id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn list_teams_for_org(&self, org_id: &str) -> Result<Vec<Team>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let teams = state
            .teams
            .values()
            .filter(|t| t.org_id == org_id)
            .cloned()
            .collect();
        Ok(teams)
    }

    async fn delete_team(&self, team_id: &str) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(team) = state.teams.remove(team_id) {
            state
                .team_by_slug
                .remove(&(team.org_id, team.normalized_slug));
        }
        Ok(())
    }

    async fn add_team_member(&self, membership: TeamMembership) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (membership.team_id.clone(), membership.user_id.clone());
        state.team_memberships.insert(key, membership);
        Ok(())
    }

    async fn remove_team_member(&self, team_id: &str, user_id: &str) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (team_id.to_string(), user_id.to_string());
        state.team_memberships.remove(&key);
        Ok(())
    }

    async fn list_team_members(&self, team_id: &str) -> Result<Vec<TeamMembership>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (perm.team_id.clone(), perm.repo_id.clone());
        state.team_repo_permissions.insert(key, perm);
        Ok(())
    }

    async fn get_team_repository_permission(
        &self,
        team_id: &str,
        repo_id: &str,
    ) -> Result<Option<TeamPermissionLevel>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (team_id.to_string(), repo_id.to_string());
        Ok(state
            .team_repo_permissions
            .get(&key)
            .map(|p| p.permission_level))
    }

    async fn list_team_repository_permissions_for_repo(
        &self,
        repo_id: &str,
    ) -> Result<Vec<TeamRepositoryPermission>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut user_teams = Vec::new();
        for (team_id, m_user_id) in state.team_memberships.keys() {
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
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state
            .milestones
            .insert(milestone.id.clone(), milestone.clone());
        Ok(milestone)
    }

    async fn get_milestone_by_id(&self, id: &str) -> Result<Option<Milestone>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state.milestones.get(id).cloned())
    }

    async fn list_milestones_for_repo(&self, repo_id: &str) -> Result<Vec<Milestone>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let list = state
            .milestones
            .values()
            .filter(|m| m.repo_id == repo_id)
            .cloned()
            .collect();
        Ok(list)
    }

    async fn update_milestone(&self, milestone: Milestone) -> Result<Milestone, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(m) = state.milestones.get_mut(&milestone.id) {
            *m = milestone.clone();
            return Ok(milestone);
        }
        Err(ForgeError::NotFound("Milestone not found".into()))
    }

    async fn delete_milestone(&self, id: &str) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.milestones.remove(id);
        Ok(())
    }

    async fn create_label(&self, label: RepositoryLabel) -> Result<RepositoryLabel, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.labels.insert(label.id.clone(), label.clone());
        Ok(label)
    }

    async fn list_labels_for_repo(
        &self,
        repo_id: &str,
    ) -> Result<Vec<RepositoryLabel>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let list = state
            .labels
            .values()
            .filter(|l| l.repo_id == repo_id)
            .cloned()
            .collect();
        Ok(list)
    }

    async fn delete_label(&self, id: &str) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.labels.remove(id);
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Issues & Issue Comments (Batch 09)
    // -------------------------------------------------------------------------
    async fn create_issue(&self, mut issue: Issue) -> Result<Issue, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let next_num = state
            .next_issue_number
            .entry(issue.repo_id.clone())
            .or_insert(0);
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
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let issues = state
            .issues
            .values()
            .filter(|i| {
                i.repo_id == repo_id && (status.is_none() || status == Some(i.status.as_str()))
            })
            .cloned()
            .collect();
        Ok(issues)
    }

    async fn update_issue(&self, issue: Issue) -> Result<Issue, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(i) = state.issues.get_mut(&issue.id) {
            *i = issue.clone();
            return Ok(issue);
        }
        Err(ForgeError::NotFound("Issue not found".into()))
    }

    async fn assign_issue(&self, issue_id: &str, user_id: &str) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (issue_id.to_string(), user_id.to_string());
        state.issue_assignees.insert(key, ());
        Ok(())
    }

    async fn unassign_issue(&self, issue_id: &str, user_id: &str) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (issue_id.to_string(), user_id.to_string());
        state.issue_assignees.remove(&key);
        Ok(())
    }

    async fn list_issue_assignees(&self, issue_id: &str) -> Result<Vec<String>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let assignees = state
            .issue_assignees
            .keys()
            .filter(|(i_id, _)| i_id == issue_id)
            .map(|(_, u_id)| u_id.clone())
            .collect();
        Ok(assignees)
    }

    async fn add_issue_label(&self, issue_id: &str, label_id: &str) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (issue_id.to_string(), label_id.to_string());
        state.issue_labels.insert(key, ());
        Ok(())
    }

    async fn remove_issue_label(&self, issue_id: &str, label_id: &str) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (issue_id.to_string(), label_id.to_string());
        state.issue_labels.remove(&key);
        Ok(())
    }

    async fn list_issue_labels(&self, issue_id: &str) -> Result<Vec<String>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let labels = state
            .issue_labels
            .keys()
            .filter(|(i_id, _)| i_id == issue_id)
            .map(|(_, l_id)| l_id.clone())
            .collect();
        Ok(labels)
    }

    async fn create_issue_comment(
        &self,
        comment: IssueComment,
    ) -> Result<IssueComment, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state
            .issue_comments
            .insert(comment.id.clone(), comment.clone());
        Ok(comment)
    }

    async fn list_issue_comments(&self, issue_id: &str) -> Result<Vec<IssueComment>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let prs = state
            .pull_requests
            .values()
            .filter(|p| {
                p.repo_id == repo_id
                    && (state_filter.is_none() || state_filter == Some(p.state.as_str()))
            })
            .cloned()
            .collect();
        Ok(prs)
    }

    async fn update_pull_request(&self, pr: PullRequest) -> Result<PullRequest, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (pr_id.to_string(), reviewer_id.to_string());
        state.pr_reviewers.insert(key, requested_by.to_string());
        Ok(())
    }

    async fn list_pr_requested_reviewers(&self, pr_id: &str) -> Result<Vec<String>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let reviewers = state
            .pr_reviewers
            .keys()
            .filter(|(p_id, _)| p_id == pr_id)
            .map(|(_, r_id)| r_id.clone())
            .collect();
        Ok(reviewers)
    }

    async fn create_pr_review(
        &self,
        review: PullRequestReview,
    ) -> Result<PullRequestReview, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.pr_reviews.insert(review.id.clone(), review.clone());
        Ok(review)
    }

    async fn list_pr_reviews(&self, pr_id: &str) -> Result<Vec<PullRequestReview>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state
            .pr_review_comments
            .insert(comment.id.clone(), comment.clone());
        Ok(comment)
    }

    async fn list_pr_review_comments(
        &self,
        pr_id: &str,
    ) -> Result<Vec<PullRequestReviewComment>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut comments: Vec<PullRequestReviewComment> = state
            .pr_review_comments
            .values()
            .filter(|c| c.pr_id == pr_id)
            .cloned()
            .collect();
        comments.sort_by_key(|c| c.created_at_nanos);
        Ok(comments)
    }

    async fn resolve_pr_review_comment(
        &self,
        comment_id: &str,
        resolved: bool,
    ) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (rule.repo_id.clone(), rule.branch_pattern.clone());
        state
            .branch_prot_by_repo_pattern
            .insert(key, rule.id.clone());
        state
            .branch_protection_rules
            .insert(rule.id.clone(), rule.clone());
        Ok(rule)
    }

    async fn get_branch_protection(
        &self,
        repo_id: &str,
        branch: &str,
    ) -> Result<Option<BranchProtectionRule>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        // Check exact match or glob wildcard match
        for r in state.branch_protection_rules.values() {
            if r.repo_id == repo_id && (r.branch_pattern == branch || r.branch_pattern == "*") {
                return Ok(Some(r.clone()));
            }
        }
        Ok(None)
    }

    async fn list_branch_protection_rules(
        &self,
        repo_id: &str,
    ) -> Result<Vec<BranchProtectionRule>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let rules = state
            .branch_protection_rules
            .values()
            .filter(|r| r.repo_id == repo_id)
            .cloned()
            .collect();
        Ok(rules)
    }

    async fn delete_branch_protection(&self, rule_id: &str) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(r) = state.branch_protection_rules.remove(rule_id) {
            state
                .branch_prot_by_repo_pattern
                .remove(&(r.repo_id, r.branch_pattern));
        }
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Notifications & Activity Events (Batch 09)
    // -------------------------------------------------------------------------
    async fn create_notification(
        &self,
        notif: UserNotification,
    ) -> Result<UserNotification, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.notifications.insert(notif.id.clone(), notif.clone());
        Ok(notif)
    }

    async fn list_user_notifications(
        &self,
        user_id: &str,
        unread_only: bool,
    ) -> Result<Vec<UserNotification>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut list: Vec<UserNotification> = state
            .notifications
            .values()
            .filter(|n| n.user_id == user_id && (!unread_only || !n.is_read))
            .cloned()
            .collect();
        list.sort_by_key(|n| std::cmp::Reverse(n.created_at_nanos));
        Ok(list)
    }

    async fn mark_notification_read(
        &self,
        notif_id: &str,
        user_id: &str,
    ) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(n) = state.notifications.get_mut(notif_id) {
            if n.user_id == user_id {
                n.is_read = true;
                return Ok(());
            }
        }
        Err(ForgeError::NotFound("Notification not found".into()))
    }

    async fn mark_all_notifications_read(&self, user_id: &str) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        for n in state.notifications.values_mut() {
            if n.user_id == user_id {
                n.is_read = true;
            }
        }
        Ok(())
    }

    async fn record_activity_event(
        &self,
        event: RepositoryActivityEvent,
    ) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.activity_events.push(event);
        Ok(())
    }

    async fn list_activity_events_for_repo(
        &self,
        repo_id: &str,
        limit: usize,
    ) -> Result<Vec<RepositoryActivityEvent>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
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

    // -------------------------------------------------------------------------
    // Batch 11: Artifacts, Releases & Packages
    // -------------------------------------------------------------------------

    async fn get_or_create_quota(
        &self,
        owner_type: &str,
        owner_id: &str,
    ) -> Result<ArtifactQuota, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(quota) = state.quotas.get(owner_id) {
            return Ok(quota.clone());
        }

        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let quota = ArtifactQuota {
            id: format!("quota_{}", uuid::Uuid::new_v4().simple()),
            owner_type: owner_type.to_string(),
            owner_id: owner_id.to_string(),
            logical_quota_bytes: 53687091200, // 50 GB
            used_logical_bytes: 0,
            used_physical_bytes: 0,
            created_at_nanos: now,
            updated_at_nanos: now,
        };
        state.quotas.insert(owner_id.to_string(), quota.clone());
        Ok(quota)
    }

    async fn update_quota_usage(
        &self,
        owner_id: &str,
        delta_logical: i64,
        delta_physical: i64,
    ) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(quota) = state.quotas.get_mut(owner_id) {
            quota.used_logical_bytes = (quota.used_logical_bytes + delta_logical).max(0);
            quota.used_physical_bytes = (quota.used_physical_bytes + delta_physical).max(0);
            quota.updated_at_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        }
        Ok(())
    }

    async fn create_upload_session(
        &self,
        session: ArtifactUploadSession,
    ) -> Result<ArtifactUploadSession, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state
            .upload_sessions
            .insert(session.id.clone(), session.clone());
        Ok(session)
    }

    async fn get_upload_session(
        &self,
        session_id: &str,
    ) -> Result<Option<ArtifactUploadSession>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state.upload_sessions.get(session_id).cloned())
    }

    async fn update_upload_session_state(
        &self,
        session_id: &str,
        new_state: &str,
    ) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(session) = state.upload_sessions.get_mut(session_id) {
            session.state = new_state.to_string();
        }
        Ok(())
    }

    async fn save_artifact_manifest(
        &self,
        manifest: StoredArtifactManifest,
    ) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state
            .manifests_by_artifact_id
            .insert(manifest.artifact_id.clone(), manifest.clone());
        state.manifests.insert(manifest.id.clone(), manifest);
        Ok(())
    }

    async fn get_artifact_manifest(
        &self,
        manifest_id: &str,
    ) -> Result<Option<StoredArtifactManifest>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state.manifests.get(manifest_id).cloned())
    }

    async fn get_artifact_manifest_by_artifact_id(
        &self,
        artifact_id: &str,
    ) -> Result<Option<StoredArtifactManifest>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state.manifests_by_artifact_id.get(artifact_id).cloned())
    }

    async fn save_lfs_object(&self, object: LfsObject) -> Result<LfsObject, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (object.repository_id.clone(), object.oid.clone());
        state.lfs_objects_by_repo_oid.insert(key, object.id.clone());
        state.lfs_objects.insert(object.id.clone(), object.clone());
        Ok(object)
    }

    async fn get_lfs_object_by_repo_oid(
        &self,
        repository_id: &str,
        oid: &str,
    ) -> Result<Option<LfsObject>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (repository_id.to_string(), oid.to_string());
        if let Some(id) = state.lfs_objects_by_repo_oid.get(&key) {
            Ok(state.lfs_objects.get(id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn get_artifact_manifest_by_repo_oid(
        &self,
        repository_id: &str,
        oid: &str,
    ) -> Result<Option<StoredArtifactManifest>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (repository_id.to_string(), oid.to_string());
        if let Some(id) = state.lfs_objects_by_repo_oid.get(&key) {
            if let Some(lfs_obj) = state.lfs_objects.get(id) {
                return Ok(state.manifests.get(&lfs_obj.manifest_id).cloned());
            }
        }
        Ok(None)
    }

    async fn delete_lfs_object(&self, repository_id: &str, oid: &str) -> Result<bool, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (repository_id.to_string(), oid.to_string());
        if let Some(id) = state.lfs_objects_by_repo_oid.remove(&key) {
            state.lfs_objects.remove(&id);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    async fn create_release(&self, release: Release) -> Result<Release, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (release.repo_id.clone(), release.tag_name.clone());
        if state.release_by_repo_tag.contains_key(&key) {
            return Err(ForgeError::Conflict(format!(
                "Release with tag '{}' already exists",
                release.tag_name
            )));
        }
        state.release_by_repo_tag.insert(key, release.id.clone());
        state.releases.insert(release.id.clone(), release.clone());
        Ok(release)
    }

    async fn get_release_by_id(&self, id: &str) -> Result<Option<Release>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state.releases.get(id).cloned())
    }

    async fn get_release_by_tag(
        &self,
        repo_id: &str,
        tag_name: &str,
    ) -> Result<Option<Release>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(id) = state
            .release_by_repo_tag
            .get(&(repo_id.to_string(), tag_name.to_string()))
        {
            Ok(state.releases.get(id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn list_releases_for_repo(
        &self,
        repo_id: &str,
        include_drafts: bool,
    ) -> Result<Vec<Release>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut rels: Vec<Release> = state
            .releases
            .values()
            .filter(|r| r.repo_id == repo_id && (include_drafts || !r.is_draft))
            .cloned()
            .collect();
        rels.sort_by_key(|r| std::cmp::Reverse(r.created_at_nanos));
        Ok(rels)
    }

    async fn update_release(&self, release: Release) -> Result<Release, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.releases.insert(release.id.clone(), release.clone());
        Ok(release)
    }

    async fn delete_release(&self, id: &str) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(r) = state.releases.remove(id) {
            state.release_by_repo_tag.remove(&(r.repo_id, r.tag_name));
            state.release_assets.retain(|_, a| a.release_id != id);
        }
        Ok(())
    }

    async fn create_release_asset(&self, asset: ReleaseAsset) -> Result<ReleaseAsset, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.release_assets.insert(asset.id.clone(), asset.clone());
        Ok(asset)
    }

    async fn get_release_asset(&self, id: &str) -> Result<Option<ReleaseAsset>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state.release_assets.get(id).cloned())
    }

    async fn list_assets_for_release(
        &self,
        release_id: &str,
    ) -> Result<Vec<ReleaseAsset>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut assets: Vec<ReleaseAsset> = state
            .release_assets
            .values()
            .filter(|a| a.release_id == release_id)
            .cloned()
            .collect();
        assets.sort_by_key(|a| a.name.clone());
        Ok(assets)
    }

    async fn delete_release_asset(&self, id: &str) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.release_assets.remove(id);
        Ok(())
    }

    async fn increment_asset_downloads(&self, id: &str) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(asset) = state.release_assets.get_mut(id) {
            asset.downloads_count += 1;
        }
        Ok(())
    }

    async fn create_artifact_namespace(
        &self,
        ns: ArtifactNamespace,
    ) -> Result<ArtifactNamespace, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let norm = ns.name.to_lowercase();
        if state.artifact_ns_by_name.contains_key(&norm) {
            return Err(ForgeError::Conflict(format!(
                "Namespace '{}' already exists",
                ns.name
            )));
        }
        state.artifact_ns_by_name.insert(norm, ns.id.clone());
        state.artifact_namespaces.insert(ns.id.clone(), ns.clone());
        Ok(ns)
    }

    async fn get_artifact_namespace_by_name(
        &self,
        name: &str,
    ) -> Result<Option<ArtifactNamespace>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(id) = state.artifact_ns_by_name.get(&name.to_lowercase()) {
            Ok(state.artifact_namespaces.get(id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn list_artifact_namespaces(&self) -> Result<Vec<ArtifactNamespace>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut list: Vec<ArtifactNamespace> =
            state.artifact_namespaces.values().cloned().collect();
        list.sort_by_key(|n| n.name.clone());
        Ok(list)
    }

    async fn create_artifact_version(
        &self,
        ver: ArtifactVersion,
    ) -> Result<ArtifactVersion, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let exists = state.artifact_versions.values().any(|v| {
            v.namespace_id == ver.namespace_id
                && v.artifact_name == ver.artifact_name
                && v.version == ver.version
        });
        if exists {
            return Err(ForgeError::Conflict(format!(
                "Artifact version '{}:{}' already exists in namespace",
                ver.artifact_name, ver.version
            )));
        }
        state.artifact_versions.insert(ver.id.clone(), ver.clone());
        Ok(ver)
    }

    async fn get_artifact_version(
        &self,
        namespace_id: &str,
        artifact_name: &str,
        version: &str,
    ) -> Result<Option<ArtifactVersion>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state
            .artifact_versions
            .values()
            .find(|v| {
                v.namespace_id == namespace_id
                    && v.artifact_name == artifact_name
                    && v.version == version
            })
            .cloned())
    }

    async fn list_artifact_versions(
        &self,
        namespace_id: &str,
        artifact_name: &str,
    ) -> Result<Vec<ArtifactVersion>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut list: Vec<ArtifactVersion> = state
            .artifact_versions
            .values()
            .filter(|v| v.namespace_id == namespace_id && v.artifact_name == artifact_name)
            .cloned()
            .collect();
        list.sort_by_key(|v| std::cmp::Reverse(v.created_at_nanos));
        Ok(list)
    }

    async fn create_package(&self, pkg: Package) -> Result<Package, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (pkg.ecosystem.clone(), pkg.name.to_lowercase());
        if state.pkg_by_eco_name.contains_key(&key) {
            return Err(ForgeError::Conflict(format!(
                "Package '{}' already exists in ecosystem '{}'",
                pkg.name, pkg.ecosystem
            )));
        }
        state.pkg_by_eco_name.insert(key, pkg.id.clone());
        state.packages.insert(pkg.id.clone(), pkg.clone());
        Ok(pkg)
    }

    async fn get_package_by_name(
        &self,
        ecosystem: &str,
        name: &str,
    ) -> Result<Option<Package>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(id) = state
            .pkg_by_eco_name
            .get(&(ecosystem.to_string(), name.to_lowercase()))
        {
            Ok(state.packages.get(id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn list_packages(&self, ecosystem: &str) -> Result<Vec<Package>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut list: Vec<Package> = state
            .packages
            .values()
            .filter(|p| p.ecosystem == ecosystem)
            .cloned()
            .collect();
        list.sort_by_key(|p| p.name.clone());
        Ok(list)
    }

    async fn create_package_version(
        &self,
        ver: PackageVersion,
    ) -> Result<PackageVersion, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let exists = state
            .package_versions
            .values()
            .any(|v| v.package_id == ver.package_id && v.version == ver.version);
        if exists {
            return Err(ForgeError::Conflict(format!(
                "Package version '{}' already exists",
                ver.version
            )));
        }
        state.package_versions.insert(ver.id.clone(), ver.clone());
        Ok(ver)
    }

    async fn get_package_version(
        &self,
        package_id: &str,
        version: &str,
    ) -> Result<Option<PackageVersion>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state
            .package_versions
            .values()
            .find(|v| v.package_id == package_id && v.version == version)
            .cloned())
    }

    async fn list_package_versions(
        &self,
        package_id: &str,
    ) -> Result<Vec<PackageVersion>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut list: Vec<PackageVersion> = state
            .package_versions
            .values()
            .filter(|v| v.package_id == package_id)
            .cloned()
            .collect();
        list.sort_by_key(|v| v.version.clone());
        Ok(list)
    }

    async fn set_package_version_yanked(
        &self,
        package_id: &str,
        version: &str,
        yanked: bool,
    ) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(v) = state
            .package_versions
            .values_mut()
            .find(|v| v.package_id == package_id && v.version == version)
        {
            v.is_yanked = yanked;
            Ok(())
        } else {
            Err(ForgeError::NotFound(format!(
                "Version '{}' not found",
                version
            )))
        }
    }

    // -------------------------------------------------------------------------
    // Webhooks (Batch 12)
    // -------------------------------------------------------------------------

    async fn create_webhook_subscription(
        &self,
        sub: WebhookSubscription,
    ) -> Result<WebhookSubscription, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state
            .webhook_subscriptions
            .insert(sub.id.clone(), sub.clone());
        Ok(sub)
    }

    async fn get_webhook_subscription(
        &self,
        id: &str,
    ) -> Result<Option<WebhookSubscription>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state.webhook_subscriptions.get(id).cloned())
    }

    async fn list_webhook_subscriptions(
        &self,
        owner_type: &str,
        owner_id: &str,
    ) -> Result<Vec<WebhookSubscription>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut list: Vec<WebhookSubscription> = state
            .webhook_subscriptions
            .values()
            .filter(|s| s.owner_type == owner_type && s.owner_id == owner_id)
            .cloned()
            .collect();
        list.sort_by_key(|s| s.created_at_nanos);
        Ok(list)
    }

    async fn update_webhook_subscription(
        &self,
        sub: WebhookSubscription,
    ) -> Result<WebhookSubscription, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if state.webhook_subscriptions.contains_key(&sub.id) {
            state
                .webhook_subscriptions
                .insert(sub.id.clone(), sub.clone());
            Ok(sub)
        } else {
            Err(ForgeError::NotFound(
                "Webhook subscription not found".into(),
            ))
        }
    }

    async fn delete_webhook_subscription(&self, id: &str) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.webhook_subscriptions.remove(id);
        Ok(())
    }

    async fn create_webhook_delivery(
        &self,
        delivery: WebhookDelivery,
    ) -> Result<WebhookDelivery, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state
            .webhook_deliveries
            .insert(delivery.id.clone(), delivery.clone());
        Ok(delivery)
    }

    async fn list_webhook_deliveries(
        &self,
        subscription_id: &str,
        limit: i64,
    ) -> Result<Vec<WebhookDelivery>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut list: Vec<WebhookDelivery> = state
            .webhook_deliveries
            .values()
            .filter(|d| d.subscription_id == subscription_id)
            .cloned()
            .collect();
        list.sort_by_key(|d| std::cmp::Reverse(d.delivered_at_nanos));
        list.truncate(limit as usize);
        Ok(list)
    }

    async fn enqueue_webhook_outbox(
        &self,
        item: WebhookOutboxItem,
    ) -> Result<WebhookOutboxItem, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.webhook_outbox.insert(item.id.clone(), item.clone());
        Ok(item)
    }

    async fn count_pending_webhook_outbox(&self, now_nanos: i64) -> Result<usize, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let count = state
            .webhook_outbox
            .values()
            .filter(|item| {
                (item.status == "PENDING" && item.next_attempt_at_nanos <= now_nanos)
                    || (item.status == "PROCESSING" && item.next_attempt_at_nanos <= now_nanos)
            })
            .count();
        Ok(count)
    }

    async fn claim_pending_webhook_outbox(
        &self,
        limit: i64,
        now_nanos: i64,
    ) -> Result<Vec<WebhookOutboxItem>, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut eligible_items: Vec<WebhookOutboxItem> = state
            .webhook_outbox
            .values()
            .filter(|item| {
                (item.status == "PENDING" && item.next_attempt_at_nanos <= now_nanos)
                    || (item.status == "PROCESSING" && item.next_attempt_at_nanos <= now_nanos)
            })
            .cloned()
            .collect();
        // Deterministic FIFO order matching PostgreSQL's ORDER BY next_attempt_at_nanos ASC, id ASC
        eligible_items.sort_by(|a, b| {
            (a.next_attempt_at_nanos, a.created_at_nanos, &a.id).cmp(&(
                b.next_attempt_at_nanos,
                b.created_at_nanos,
                &b.id,
            ))
        });
        eligible_items.truncate(limit as usize);

        let mut claimed = Vec::new();
        for (idx, eligible) in eligible_items.into_iter().enumerate() {
            if let Some(item) = state.webhook_outbox.get_mut(&eligible.id) {
                item.status = "PROCESSING".into();
                item.attempts += 1;
                // Base 60s lease plus 30s per position in batch to prevent premature reclaim
                let item_lease_secs = 60 + (idx as i64) * 30;
                item.next_attempt_at_nanos = now_nanos + item_lease_secs * 1_000_000_000;
                claimed.push(item.clone());
            }
        }
        Ok(claimed)
    }

    async fn extend_webhook_outbox_lease(
        &self,
        id: &str,
        lease_until_nanos: i64,
    ) -> Result<bool, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(item) = state.webhook_outbox.get_mut(id) {
            if item.status == "PROCESSING" {
                item.next_attempt_at_nanos = lease_until_nanos;
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn update_webhook_outbox_status(
        &self,
        id: &str,
        status: &str,
        error: Option<&str>,
        next_attempt_nanos: i64,
    ) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(item) = state.webhook_outbox.get_mut(id) {
            item.status = status.to_string();
            item.last_error = error.map(|s| s.to_string());
            item.next_attempt_at_nanos = next_attempt_nanos;
            Ok(())
        } else {
            Err(ForgeError::NotFound("Outbox item not found".into()))
        }
    }

    // -------------------------------------------------------------------------
    // Workflows, Runs, Jobs, Steps, Logs (Batch 12)
    // -------------------------------------------------------------------------

    async fn upsert_workflow_definition(
        &self,
        def: WorkflowDefinition,
    ) -> Result<WorkflowDefinition, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = format!("{}:{}", def.repo_id, def.path);
        state.workflow_definitions.insert(key, def.clone());
        Ok(def)
    }

    async fn get_workflow_definition(
        &self,
        repo_id: &str,
        path: &str,
    ) -> Result<Option<WorkflowDefinition>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = format!("{}:{}", repo_id, path);
        Ok(state.workflow_definitions.get(&key).cloned())
    }

    async fn list_workflow_definitions(
        &self,
        repo_id: &str,
    ) -> Result<Vec<WorkflowDefinition>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut list: Vec<WorkflowDefinition> = state
            .workflow_definitions
            .values()
            .filter(|d| d.repo_id == repo_id)
            .cloned()
            .collect();
        list.sort_by_key(|d| d.path.clone());
        Ok(list)
    }

    async fn create_workflow_run(&self, run: WorkflowRun) -> Result<WorkflowRun, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.workflow_runs.insert(run.id.clone(), run.clone());
        Ok(run)
    }

    async fn get_workflow_run(&self, id: &str) -> Result<Option<WorkflowRun>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state.workflow_runs.get(id).cloned())
    }

    async fn list_workflow_runs(
        &self,
        repo_id: &str,
        limit: i64,
    ) -> Result<Vec<WorkflowRun>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut list: Vec<WorkflowRun> = state
            .workflow_runs
            .values()
            .filter(|r| r.repo_id == repo_id)
            .cloned()
            .collect();
        list.sort_by_key(|r| std::cmp::Reverse(r.created_at_nanos));
        list.truncate(limit as usize);
        Ok(list)
    }

    async fn list_active_workflow_runs_for_repo(
        &self,
        repo_id: &str,
    ) -> Result<Vec<WorkflowRun>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut list: Vec<WorkflowRun> = state
            .workflow_runs
            .values()
            .filter(|r| r.repo_id == repo_id && r.status != "COMPLETED" && r.status != "CANCELLED")
            .cloned()
            .collect();
        // FIFO order ensures older runs are scheduled first without starvation
        list.sort_by_key(|r| r.created_at_nanos);
        Ok(list)
    }

    async fn list_active_workflow_repo_ids(&self) -> Result<Vec<String>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut repos: Vec<String> = state
            .workflow_runs
            .values()
            .filter(|r| r.status != "COMPLETED" && r.status != "CANCELLED")
            .map(|r| r.repo_id.clone())
            .collect();
        repos.sort();
        repos.dedup();
        Ok(repos)
    }

    async fn update_workflow_run_status(
        &self,
        id: &str,
        status: &str,
        conclusion: Option<&str>,
        started_at: Option<i64>,
        completed_at: Option<i64>,
    ) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(r) = state.workflow_runs.get_mut(id) {
            r.status = status.to_string();
            if let Some(c) = conclusion {
                r.conclusion = Some(c.to_string());
            }
            if started_at.is_some() {
                r.started_at_nanos = started_at;
            }
            if completed_at.is_some() {
                r.completed_at_nanos = completed_at;
            }
            Ok(())
        } else {
            Err(ForgeError::NotFound("Workflow run not found".into()))
        }
    }

    async fn create_workflow_job(&self, job: WorkflowJob) -> Result<WorkflowJob, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.workflow_jobs.insert(job.id.clone(), job.clone());
        Ok(job)
    }

    async fn get_workflow_job(&self, id: &str) -> Result<Option<WorkflowJob>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state.workflow_jobs.get(id).cloned())
    }

    async fn list_workflow_jobs(&self, run_id: &str) -> Result<Vec<WorkflowJob>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut list: Vec<WorkflowJob> = state
            .workflow_jobs
            .values()
            .filter(|j| j.run_id == run_id)
            .cloned()
            .collect();
        list.sort_by_key(|j| j.created_at_nanos);
        Ok(list)
    }

    async fn list_queued_workflow_jobs(
        &self,
        now_nanos: i64,
    ) -> Result<Vec<WorkflowJob>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut list: Vec<WorkflowJob> = state
            .workflow_jobs
            .values()
            .filter(|j| {
                j.status == "QUEUED"
                    || (j.status == "ASSIGNED"
                        && j.lease_expires_at_nanos.is_some_and(|exp| exp < now_nanos))
            })
            .cloned()
            .collect();
        list.sort_by_key(|j| j.created_at_nanos);
        Ok(list)
    }

    async fn assign_workflow_job(
        &self,
        job_id: &str,
        runner_id: &str,
        fencing_token: i64,
        lease_expires_at: i64,
    ) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(j) = state.workflow_jobs.get_mut(job_id) {
            j.status = "ASSIGNED".into();
            j.runner_id = Some(runner_id.to_string());
            j.fencing_token = fencing_token;
            j.lease_expires_at_nanos = Some(lease_expires_at);
            Ok(())
        } else {
            Err(ForgeError::NotFound("Workflow job not found".into()))
        }
    }

    async fn update_workflow_job_status(
        &self,
        job_id: &str,
        status: &str,
        exit_code: Option<i32>,
        started_at: Option<i64>,
        completed_at: Option<i64>,
    ) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(j) = state.workflow_jobs.get_mut(job_id) {
            j.status = status.to_string();
            if exit_code.is_some() {
                j.exit_code = exit_code;
            }
            if started_at.is_some() {
                j.started_at_nanos = started_at;
            }
            if completed_at.is_some() {
                j.completed_at_nanos = completed_at;
            }
            Ok(())
        } else {
            Err(ForgeError::NotFound("Workflow job not found".into()))
        }
    }

    async fn create_workflow_step(&self, step: WorkflowStep) -> Result<WorkflowStep, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.workflow_steps.insert(step.id.clone(), step.clone());
        Ok(step)
    }

    async fn list_workflow_steps(&self, job_id: &str) -> Result<Vec<WorkflowStep>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut list: Vec<WorkflowStep> = state
            .workflow_steps
            .values()
            .filter(|s| s.job_id == job_id)
            .cloned()
            .collect();
        list.sort_by_key(|s| s.step_index);
        Ok(list)
    }

    async fn update_workflow_step(
        &self,
        step_id: &str,
        status: &str,
        exit_code: Option<i32>,
        started_at: Option<i64>,
        completed_at: Option<i64>,
    ) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(s) = state.workflow_steps.get_mut(step_id) {
            s.status = status.to_string();
            if exit_code.is_some() {
                s.exit_code = exit_code;
            }
            if started_at.is_some() {
                s.started_at_nanos = started_at;
            }
            if completed_at.is_some() {
                s.completed_at_nanos = completed_at;
            }
            Ok(())
        } else {
            Err(ForgeError::NotFound("Workflow step not found".into()))
        }
    }

    async fn append_workflow_log(&self, log: WorkflowLogChunk) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.workflow_logs.push(log);
        Ok(())
    }

    async fn list_workflow_logs(&self, job_id: &str) -> Result<Vec<WorkflowLogChunk>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut list: Vec<WorkflowLogChunk> = state
            .workflow_logs
            .iter()
            .filter(|l| l.job_id == job_id)
            .cloned()
            .collect();
        list.sort_by_key(|l| l.timestamp_nanos);
        Ok(list)
    }

    // -------------------------------------------------------------------------
    // Runners (Batch 12)
    // -------------------------------------------------------------------------

    async fn register_runner(
        &self,
        runner: RunnerRegistration,
    ) -> Result<RunnerRegistration, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state
            .runner_by_token_hash
            .insert(runner.token_hash.clone(), runner.id.clone());
        state.runners.insert(runner.id.clone(), runner.clone());
        Ok(runner)
    }

    async fn get_runner_by_id(&self, id: &str) -> Result<Option<RunnerRegistration>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state.runners.get(id).cloned())
    }

    async fn get_runner_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<RunnerRegistration>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(id) = state.runner_by_token_hash.get(token_hash) {
            Ok(state.runners.get(id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn list_runners(&self) -> Result<Vec<RunnerRegistration>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut list: Vec<RunnerRegistration> = state.runners.values().cloned().collect();
        list.sort_by_key(|r| r.name.clone());
        Ok(list)
    }

    async fn update_runner_heartbeat(
        &self,
        runner_id: &str,
        active_jobs: i32,
        now_nanos: i64,
    ) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(r) = state.runners.get_mut(runner_id) {
            r.active_jobs = active_jobs;
            r.last_heartbeat_at_nanos = now_nanos;
            Ok(())
        } else {
            Err(ForgeError::NotFound("Runner not found".into()))
        }
    }

    async fn update_runner_status(&self, runner_id: &str, status: &str) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(r) = state.runners.get_mut(runner_id) {
            r.status = status.to_string();
            Ok(())
        } else {
            Err(ForgeError::NotFound("Runner not found".into()))
        }
    }

    // -------------------------------------------------------------------------
    // Workflow Secrets & PR Check Results (Batch 12)
    // -------------------------------------------------------------------------

    async fn upsert_workflow_secret(
        &self,
        secret: WorkflowSecret,
    ) -> Result<WorkflowSecret, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = format!("{}:{}:{}", secret.owner_type, secret.owner_id, secret.name);
        state.workflow_secrets.insert(key, secret.clone());
        Ok(secret)
    }

    async fn get_workflow_secret(
        &self,
        owner_type: &str,
        owner_id: &str,
        name: &str,
    ) -> Result<Option<WorkflowSecret>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = format!("{}:{}:{}", owner_type, owner_id, name);
        Ok(state.workflow_secrets.get(&key).cloned())
    }

    async fn list_workflow_secrets(
        &self,
        owner_type: &str,
        owner_id: &str,
    ) -> Result<Vec<WorkflowSecret>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut list: Vec<WorkflowSecret> = state
            .workflow_secrets
            .values()
            .filter(|s| s.owner_type == owner_type && s.owner_id == owner_id)
            .cloned()
            .collect();
        list.sort_by_key(|s| s.name.clone());
        Ok(list)
    }

    async fn delete_workflow_secret(
        &self,
        owner_type: &str,
        owner_id: &str,
        name: &str,
    ) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = format!("{}:{}:{}", owner_type, owner_id, name);
        state.workflow_secrets.remove(&key);
        Ok(())
    }

    async fn upsert_workflow_check_result(
        &self,
        check: WorkflowCheckResult,
    ) -> Result<WorkflowCheckResult, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (
            check.repo_id.clone(),
            check.commit_oid.clone(),
            check.check_name.clone(),
        );
        state.workflow_check_results.insert(key, check.clone());
        Ok(check)
    }

    async fn get_workflow_check_result(
        &self,
        repo_id: &str,
        commit_oid: &str,
        check_name: &str,
    ) -> Result<Option<WorkflowCheckResult>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (
            repo_id.to_string(),
            commit_oid.to_string(),
            check_name.to_string(),
        );
        Ok(state.workflow_check_results.get(&key).cloned())
    }

    async fn list_workflow_check_results(
        &self,
        repo_id: &str,
        commit_oid: &str,
    ) -> Result<Vec<WorkflowCheckResult>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut list: Vec<WorkflowCheckResult> = state
            .workflow_check_results
            .values()
            .filter(|c| c.repo_id == repo_id && c.commit_oid == commit_oid)
            .cloned()
            .collect();
        list.sort_by_key(|c| c.check_name.clone());
        Ok(list)
    }

    // -------------------------------------------------------------------------
    // Discussions (Batch 13)
    // -------------------------------------------------------------------------
    async fn create_discussion_category(
        &self,
        cat: DiscussionCategory,
    ) -> Result<DiscussionCategory, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state
            .discussion_categories
            .insert(cat.id.clone(), cat.clone());
        Ok(cat)
    }

    async fn get_discussion_category(
        &self,
        id: &str,
    ) -> Result<Option<DiscussionCategory>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state.discussion_categories.get(id).cloned())
    }

    async fn list_discussion_categories(
        &self,
        repo_id: &str,
    ) -> Result<Vec<DiscussionCategory>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut list: Vec<DiscussionCategory> = state
            .discussion_categories
            .values()
            .filter(|c| c.repo_id == repo_id)
            .cloned()
            .collect();
        list.sort_by_key(|c| c.created_at_nanos);
        Ok(list)
    }

    async fn create_discussion(&self, mut disc: Discussion) -> Result<Discussion, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let next_num = state
            .next_discussion_number
            .entry(disc.repo_id.clone())
            .or_insert(1);
        disc.number = *next_num;
        *next_num += 1;

        state
            .discussion_by_repo_num
            .insert((disc.repo_id.clone(), disc.number), disc.id.clone());
        state.discussions.insert(disc.id.clone(), disc.clone());
        Ok(disc)
    }

    async fn get_discussion(&self, id: &str) -> Result<Option<Discussion>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state.discussions.get(id).cloned())
    }

    async fn get_discussion_by_number(
        &self,
        repo_id: &str,
        number: i32,
    ) -> Result<Option<Discussion>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(id) = state
            .discussion_by_repo_num
            .get(&(repo_id.to_string(), number))
        {
            Ok(state.discussions.get(id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn list_discussions(
        &self,
        repo_id: &str,
        category_id: Option<&str>,
    ) -> Result<Vec<Discussion>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut list: Vec<Discussion> = state
            .discussions
            .values()
            .filter(|d| d.repo_id == repo_id && category_id.is_none_or(|c| d.category_id == c))
            .cloned()
            .collect();
        list.sort_by(|a, b| {
            b.is_pinned
                .cmp(&a.is_pinned)
                .then_with(|| b.created_at_nanos.cmp(&a.created_at_nanos))
        });
        Ok(list)
    }

    async fn update_discussion(&self, disc: Discussion) -> Result<Discussion, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.discussions.insert(disc.id.clone(), disc.clone());
        Ok(disc)
    }

    async fn delete_discussion(&self, id: &str) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(disc) = state.discussions.remove(id) {
            state
                .discussion_by_repo_num
                .remove(&(disc.repo_id, disc.number));
        }
        Ok(())
    }

    async fn create_discussion_comment(
        &self,
        comment: DiscussionComment,
    ) -> Result<DiscussionComment, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(d) = state.discussions.get_mut(&comment.discussion_id) {
            d.comments_count += 1;
        }
        state
            .discussion_comments
            .insert(comment.id.clone(), comment.clone());
        Ok(comment)
    }

    async fn get_discussion_comment(
        &self,
        id: &str,
    ) -> Result<Option<DiscussionComment>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state.discussion_comments.get(id).cloned())
    }

    async fn list_discussion_comments(
        &self,
        discussion_id: &str,
    ) -> Result<Vec<DiscussionComment>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut list: Vec<DiscussionComment> = state
            .discussion_comments
            .values()
            .filter(|c| c.discussion_id == discussion_id)
            .cloned()
            .collect();
        list.sort_by_key(|c| c.created_at_nanos);
        Ok(list)
    }

    async fn update_discussion_comment(
        &self,
        comment: DiscussionComment,
    ) -> Result<DiscussionComment, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state
            .discussion_comments
            .insert(comment.id.clone(), comment.clone());
        Ok(comment)
    }

    async fn delete_discussion_comment(&self, id: &str) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(comment) = state.discussion_comments.remove(id) {
            if let Some(d) = state.discussions.get_mut(&comment.discussion_id) {
                if d.comments_count > 0 {
                    d.comments_count -= 1;
                }
            }
        }
        Ok(())
    }

    async fn add_discussion_reaction(
        &self,
        reaction: DiscussionReaction,
    ) -> Result<DiscussionReaction, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        // Check for existing reaction
        if !state.discussion_reactions.iter().any(|r| {
            r.subject_type == reaction.subject_type
                && r.subject_id == reaction.subject_id
                && r.user_id == reaction.user_id
                && r.reaction == reaction.reaction
        }) {
            state.discussion_reactions.push(reaction.clone());
        }
        Ok(reaction)
    }

    async fn remove_discussion_reaction(
        &self,
        subject_type: &str,
        subject_id: &str,
        user_id: &str,
        reaction: &str,
    ) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.discussion_reactions.retain(|r| {
            !(r.subject_type == subject_type
                && r.subject_id == subject_id
                && r.user_id == user_id
                && r.reaction == reaction)
        });
        Ok(())
    }

    async fn list_discussion_reactions(
        &self,
        subject_type: &str,
        subject_id: &str,
    ) -> Result<Vec<DiscussionReaction>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let list: Vec<DiscussionReaction> = state
            .discussion_reactions
            .iter()
            .filter(|r| r.subject_type == subject_type && r.subject_id == subject_id)
            .cloned()
            .collect();
        Ok(list)
    }

    async fn set_discussion_subscription(
        &self,
        sub: DiscussionSubscription,
    ) -> Result<DiscussionSubscription, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.discussion_subscriptions.insert(
            (sub.user_id.clone(), sub.discussion_id.clone()),
            sub.clone(),
        );
        Ok(sub)
    }

    async fn get_discussion_subscription(
        &self,
        user_id: &str,
        discussion_id: &str,
    ) -> Result<Option<DiscussionSubscription>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state
            .discussion_subscriptions
            .get(&(user_id.to_string(), discussion_id.to_string()))
            .cloned())
    }

    // -------------------------------------------------------------------------
    // Discovery, Topics, Languages, Profiles (Batch 13)
    // -------------------------------------------------------------------------
    async fn set_repository_topics(
        &self,
        repo_id: &str,
        topics: &[String],
    ) -> Result<Vec<RepositoryTopic>, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let list: Vec<RepositoryTopic> = topics
            .iter()
            .map(|t| RepositoryTopic {
                id: uuid::Uuid::new_v4().to_string(),
                repo_id: repo_id.to_string(),
                topic: t.to_lowercase(),
                created_at_nanos: now,
            })
            .collect();
        state
            .repository_topics
            .insert(repo_id.to_string(), list.clone());
        Ok(list)
    }

    async fn list_repository_topics(
        &self,
        repo_id: &str,
    ) -> Result<Vec<RepositoryTopic>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state
            .repository_topics
            .get(repo_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn list_popular_topics(&self, limit: usize) -> Result<Vec<PopularTopicDto>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut counts: HashMap<String, usize> = HashMap::new();
        for list in state.repository_topics.values() {
            for item in list {
                *counts.entry(item.topic.clone()).or_insert(0) += 1;
            }
        }
        let mut sorted: Vec<PopularTopicDto> = counts
            .into_iter()
            .map(|(topic, count)| PopularTopicDto {
                topic,
                repository_count: count,
            })
            .collect();
        sorted.sort_by_key(|b| std::cmp::Reverse(b.repository_count));
        sorted.truncate(limit);
        Ok(sorted)
    }

    async fn list_repositories_by_topic(
        &self,
        topic: &str,
    ) -> Result<Vec<ForgeRepository>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let topic_lower = topic.to_lowercase();
        let mut matching_repo_ids = Vec::new();
        for (repo_id, list) in &state.repository_topics {
            if list.iter().any(|t| t.topic == topic_lower) {
                matching_repo_ids.push(repo_id.clone());
            }
        }

        let repos: Vec<ForgeRepository> = matching_repo_ids
            .iter()
            .filter_map(|id| state.repositories.get(id).cloned())
            .collect();
        Ok(repos)
    }

    async fn set_repository_languages(
        &self,
        repo_id: &str,
        langs: &[RepositoryLanguage],
    ) -> Result<Vec<RepositoryLanguage>, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state
            .repository_languages
            .insert(repo_id.to_string(), langs.to_vec());
        Ok(langs.to_vec())
    }

    async fn list_repository_languages(
        &self,
        repo_id: &str,
    ) -> Result<Vec<RepositoryLanguage>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state
            .repository_languages
            .get(repo_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn get_extended_user_profile(
        &self,
        user_id: &str,
    ) -> Result<Option<ExtendedUserProfile>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state.user_profiles.get(user_id).cloned())
    }

    async fn upsert_extended_user_profile(
        &self,
        profile: ExtendedUserProfile,
    ) -> Result<ExtendedUserProfile, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state
            .user_profiles
            .insert(profile.user_id.clone(), profile.clone());
        Ok(profile)
    }

    // -------------------------------------------------------------------------
    // Security Center (Batch 13)
    // -------------------------------------------------------------------------
    async fn create_security_scan(&self, scan: SecurityScan) -> Result<SecurityScan, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.security_scans.insert(scan.id.clone(), scan.clone());
        Ok(scan)
    }

    async fn list_security_scans(&self, repo_id: &str) -> Result<Vec<SecurityScan>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut list: Vec<SecurityScan> = state
            .security_scans
            .values()
            .filter(|s| s.repo_id == repo_id)
            .cloned()
            .collect();
        list.sort_by_key(|b| std::cmp::Reverse(b.started_at_nanos));
        Ok(list)
    }

    async fn create_secret_scan_finding(
        &self,
        finding: SecretScanFinding,
    ) -> Result<SecretScanFinding, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state
            .secret_scan_findings
            .insert(finding.id.clone(), finding.clone());
        Ok(finding)
    }

    async fn list_secret_scan_findings(
        &self,
        repo_id: &str,
        status_filter: Option<&str>,
    ) -> Result<Vec<SecretScanFinding>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut list: Vec<SecretScanFinding> = state
            .secret_scan_findings
            .values()
            .filter(|f| f.repo_id == repo_id && status_filter.is_none_or(|s| f.status == s))
            .cloned()
            .collect();
        list.sort_by_key(|b| std::cmp::Reverse(b.created_at_nanos));
        Ok(list)
    }

    async fn update_secret_scan_finding_status(
        &self,
        id: &str,
        status: &str,
        now_nanos: i64,
    ) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(f) = state.secret_scan_findings.get_mut(id) {
            f.status = status.to_string();
            f.resolved_at_nanos = Some(now_nanos);
        }
        Ok(())
    }

    async fn upsert_dependency_inventory(
        &self,
        items: &[DependencyInventory],
    ) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        for item in items {
            state
                .dependency_inventories
                .insert(item.id.clone(), item.clone());
        }
        Ok(())
    }

    async fn list_dependency_inventory(
        &self,
        repo_id: &str,
    ) -> Result<Vec<DependencyInventory>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut list: Vec<DependencyInventory> = state
            .dependency_inventories
            .values()
            .filter(|d| d.repo_id == repo_id)
            .cloned()
            .collect();
        list.sort_by_key(|d| d.package_name.clone());
        Ok(list)
    }

    async fn upsert_security_advisory(
        &self,
        advisory: SecurityAdvisory,
    ) -> Result<SecurityAdvisory, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state
            .security_advisories
            .insert(advisory.advisory_id.clone(), advisory.clone());
        Ok(advisory)
    }

    async fn list_security_advisories(
        &self,
        ecosystem: &str,
    ) -> Result<Vec<SecurityAdvisory>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let list: Vec<SecurityAdvisory> = state
            .security_advisories
            .values()
            .filter(|a| a.ecosystem == ecosystem)
            .cloned()
            .collect();
        Ok(list)
    }

    async fn create_dependency_vulnerability(
        &self,
        vuln: DependencyVulnerability,
    ) -> Result<DependencyVulnerability, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state
            .dependency_vulnerabilities
            .insert(vuln.id.clone(), vuln.clone());
        Ok(vuln)
    }

    async fn list_dependency_vulnerabilities(
        &self,
        repo_id: &str,
        status_filter: Option<&str>,
    ) -> Result<Vec<DependencyVulnerability>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut list: Vec<DependencyVulnerability> = state
            .dependency_vulnerabilities
            .values()
            .filter(|v| v.repo_id == repo_id && status_filter.is_none_or(|s| v.status == s))
            .cloned()
            .collect();
        list.sort_by_key(|b| std::cmp::Reverse(b.created_at_nanos));
        Ok(list)
    }

    async fn update_dependency_vulnerability_status(
        &self,
        id: &str,
        status: &str,
        now_nanos: i64,
    ) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(v) = state.dependency_vulnerabilities.get_mut(id) {
            v.status = status.to_string();
            v.resolved_at_nanos = Some(now_nanos);
        }
        Ok(())
    }

    // -------------------------------------------------------------------------
    // SSH Keys & Deploy Keys (Batch 18)
    // -------------------------------------------------------------------------
    async fn create_user_ssh_key(&self, key: UserSshKey) -> Result<UserSshKey, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if state.user_ssh_by_fingerprint.contains_key(&key.fingerprint) {
            return Err(ForgeError::Conflict(
                "An SSH key with this fingerprint already exists".to_string(),
            ));
        }
        state
            .user_ssh_by_fingerprint
            .insert(key.fingerprint.clone(), key.id.clone());
        state.user_ssh_keys.insert(key.id.clone(), key.clone());
        Ok(key)
    }

    async fn get_user_ssh_key_by_id(&self, id: &str) -> Result<Option<UserSshKey>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state.user_ssh_keys.get(id).cloned())
    }

    async fn get_user_ssh_key_by_fingerprint(
        &self,
        fingerprint: &str,
    ) -> Result<Option<UserSshKey>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(id) = state.user_ssh_by_fingerprint.get(fingerprint) {
            Ok(state.user_ssh_keys.get(id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn list_user_ssh_keys(&self, user_id: &str) -> Result<Vec<UserSshKey>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut keys: Vec<UserSshKey> = state
            .user_ssh_keys
            .values()
            .filter(|k| k.user_id == user_id && !k.revoked)
            .cloned()
            .collect();
        keys.sort_by_key(|b| std::cmp::Reverse(b.created_at_nanos));
        Ok(keys)
    }

    async fn update_user_ssh_key_last_used(
        &self,
        id: &str,
        now_nanos: i64,
    ) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(k) = state.user_ssh_keys.get_mut(id) {
            k.last_used_at_nanos = Some(now_nanos);
        }
        Ok(())
    }

    async fn revoke_user_ssh_key(&self, id: &str, user_id: &str) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let fingerprint = if let Some(k) = state.user_ssh_keys.get_mut(id) {
            if k.user_id == user_id {
                k.revoked = true;
                Some(k.fingerprint.clone())
            } else {
                None
            }
        } else {
            None
        };
        if let Some(fp) = fingerprint {
            state.user_ssh_by_fingerprint.remove(&fp);
        }
        Ok(())
    }

    async fn create_deploy_key(&self, key: DeployKey) -> Result<DeployKey, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if state.deploy_by_fingerprint.contains_key(&key.fingerprint) {
            return Err(ForgeError::Conflict(
                "A deploy key with this fingerprint already exists".to_string(),
            ));
        }
        state
            .deploy_by_fingerprint
            .insert(key.fingerprint.clone(), key.id.clone());
        state.deploy_keys.insert(key.id.clone(), key.clone());
        Ok(key)
    }

    async fn get_deploy_key_by_id(&self, id: &str) -> Result<Option<DeployKey>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state.deploy_keys.get(id).cloned())
    }

    async fn get_deploy_key_by_fingerprint(
        &self,
        fingerprint: &str,
    ) -> Result<Option<DeployKey>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(id) = state.deploy_by_fingerprint.get(fingerprint) {
            Ok(state.deploy_keys.get(id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn list_deploy_keys(&self, repo_id: &str) -> Result<Vec<DeployKey>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut keys: Vec<DeployKey> = state
            .deploy_keys
            .values()
            .filter(|k| k.repository_id == repo_id && !k.revoked)
            .cloned()
            .collect();
        keys.sort_by_key(|b| std::cmp::Reverse(b.created_at_nanos));
        Ok(keys)
    }

    async fn update_deploy_key_last_used(
        &self,
        id: &str,
        now_nanos: i64,
    ) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(k) = state.deploy_keys.get_mut(id) {
            k.last_used_at_nanos = Some(now_nanos);
        }
        Ok(())
    }

    async fn revoke_deploy_key(&self, id: &str, repo_id: &str) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let fingerprint = if let Some(k) = state.deploy_keys.get_mut(id) {
            if k.repository_id == repo_id {
                k.revoked = true;
                Some(k.fingerprint.clone())
            } else {
                None
            }
        } else {
            None
        };
        if let Some(fp) = fingerprint {
            state.deploy_by_fingerprint.remove(&fp);
        }
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Git LFS File Locks (Batch 18)
    // -------------------------------------------------------------------------
    async fn create_lfs_lock(&self, lock: LfsLock) -> Result<LfsLock, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (lock.repository_id.clone(), lock.path.clone());
        if let Some(existing_id) = state.lfs_locks_by_repo_path.get(&key) {
            if let Some(existing_lock) = state.lfs_locks.get(existing_id) {
                return Err(ForgeError::Conflict(format!(
                    "Path '{}' is already locked by {}",
                    lock.path, existing_lock.owner_username
                )));
            }
        }
        state.lfs_locks_by_repo_path.insert(key, lock.id.clone());
        state.lfs_locks.insert(lock.id.clone(), lock.clone());
        Ok(lock)
    }

    async fn get_lfs_lock_by_id(&self, id: &str) -> Result<Option<LfsLock>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        Ok(state.lfs_locks.get(id).cloned())
    }

    async fn get_lfs_lock_by_path(
        &self,
        repo_id: &str,
        path: &str,
    ) -> Result<Option<LfsLock>, ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let key = (repo_id.to_string(), path.to_string());
        if let Some(id) = state.lfs_locks_by_repo_path.get(&key) {
            Ok(state.lfs_locks.get(id).cloned())
        } else {
            Ok(None)
        }
    }

    async fn list_lfs_locks(
        &self,
        repo_id: &str,
        path: Option<&str>,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<(Vec<LfsLock>, Option<String>), ForgeError> {
        let state = self
            .state
            .read()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let mut locks: Vec<LfsLock> = state
            .lfs_locks
            .values()
            .filter(|l| l.repository_id == repo_id && path.is_none_or(|p| l.path == p))
            .cloned()
            .collect();
        locks.sort_by(|a, b| a.path.cmp(&b.path));

        let offset = cursor.and_then(|c| c.parse::<usize>().ok()).unwrap_or(0);
        let max_items = limit.unwrap_or(100).min(1000);

        let sliced: Vec<LfsLock> = locks.iter().skip(offset).take(max_items).cloned().collect();
        let next_cursor = if offset + max_items < locks.len() {
            Some((offset + max_items).to_string())
        } else {
            None
        };

        Ok((sliced, next_cursor))
    }

    async fn delete_lfs_lock(&self, id: &str) -> Result<Option<LfsLock>, ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(lock) = state.lfs_locks.remove(id) {
            state
                .lfs_locks_by_repo_path
                .remove(&(lock.repository_id.clone(), lock.path.clone()));
            Ok(Some(lock))
        } else {
            Ok(None)
        }
    }

    async fn create_lfs_lock_audit(&self, audit: LfsLockAudit) -> Result<(), ForgeError> {
        let mut state = self
            .state
            .write()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.lfs_lock_audits.push(audit);
        Ok(())
    }
}
