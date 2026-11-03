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
    audit_events: Vec<AuditEvent>,
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
        if let Some(user) = state.users.get_mut(user_id) {
            user.last_login_nanos = Some(now_nanos);
            user.updated_at_nanos = now_nanos;
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
        if let Some(user) = state.users.get_mut(user_id) {
            user.password_hash = new_hash.to_string();
            user.updated_at_nanos = now_nanos;
            Ok(())
        } else {
            Err(ForgeError::NotFound("User not found".into()))
        }
    }

    async fn create_session(&self, session: UserSession) -> Result<UserSession, ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.session_by_hash.insert(session.session_token_hash.clone(), session.id.clone());
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
        if let Some(session) = state.sessions.get_mut(session_id) {
            session.revoked = true;
        }
        Ok(())
    }

    async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        for session in state.sessions.values_mut() {
            if session.user_id == user_id {
                session.revoked = true;
            }
        }
        Ok(())
    }

    async fn list_user_sessions(&self, user_id: &str) -> Result<Vec<UserSession>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let list = state
            .sessions
            .values()
            .filter(|s| s.user_id == user_id && !s.revoked)
            .cloned()
            .collect();
        Ok(list)
    }

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

    async fn list_public_repositories(&self) -> Result<Vec<ForgeRepository>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        let repos = state
            .repositories
            .values()
            .filter(|r| !r.is_private && !r.archived)
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
            Ok(())
        } else {
            Err(ForgeError::NotFound("Repository not found".into()))
        }
    }

    async fn delete_repository(&self, repo_id: &str) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(r) = state.repositories.get_mut(repo_id) {
            r.archived = true;
            r.provisioning_status = ProvisioningStatus::Tombstoned;
            let key = format!("{}/{}", r.owner_username.to_lowercase(), r.normalized_name);
            state.repo_by_owner_name.remove(&key);
            Ok(())
        } else {
            Err(ForgeError::NotFound("Repository not found".into()))
        }
    }

    async fn create_permission(
        &self,
        perm: RepositoryPermission,
    ) -> Result<RepositoryPermission, ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.perm_by_repo_user.insert(
            (perm.repo_id.clone(), perm.user_id.clone()),
            perm.id.clone(),
        );
        state.permissions.insert(perm.id.clone(), perm.clone());
        Ok(perm)
    }

    async fn get_permission(
        &self,
        repo_id: &str,
        user_id: &str,
    ) -> Result<Option<PermissionRole>, ForgeError> {
        let state = self.state.read().map_err(|e| ForgeError::Internal(e.to_string()))?;
        if let Some(pid) = state
            .perm_by_repo_user
            .get(&(repo_id.to_string(), user_id.to_string()))
        {
            if let Some(perm) = state.permissions.get(pid) {
                return Ok(Some(perm.role));
            }
        }
        Ok(None)
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
        if let Some(pid) = state
            .perm_by_repo_user
            .remove(&(repo_id.to_string(), user_id.to_string()))
        {
            state.permissions.remove(&pid);
        }
        Ok(())
    }

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
        let mut pending: Vec<_> = state
            .provisioning_jobs
            .values()
            .filter(|j| j.status == ProvisioningStatus::Pending || j.status == ProvisioningStatus::Provisioning)
            .cloned()
            .collect();
        pending.sort_by_key(|j| j.created_at_nanos);
        Ok(pending.into_iter().next())
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
            j.attempts += 1;
            j.updated_at_nanos = now_nanos;
        }
        Ok(())
    }

    async fn record_audit_event(&self, event: AuditEvent) -> Result<(), ForgeError> {
        let mut state = self.state.write().map_err(|e| ForgeError::Internal(e.to_string()))?;
        state.audit_events.push(event);
        Ok(())
    }
}
