use crate::core::error::ForgeError;
use crate::core::types::*;
use async_trait::async_trait;

#[async_trait]
pub trait Database: Send + Sync {
    // Users
    async fn create_user(&self, user: User) -> Result<User, ForgeError>;
    async fn get_user_by_id(&self, id: &str) -> Result<Option<User>, ForgeError>;
    async fn get_user_by_username(&self, username: &str) -> Result<Option<User>, ForgeError>;
    async fn get_user_by_email(&self, email: &str) -> Result<Option<User>, ForgeError>;
    async fn update_user_last_login(&self, user_id: &str, now_nanos: i64) -> Result<(), ForgeError>;
    async fn update_user_password(
        &self,
        user_id: &str,
        new_hash: &str,
        now_nanos: i64,
    ) -> Result<(), ForgeError>;

    // Sessions
    async fn create_session(&self, session: UserSession) -> Result<UserSession, ForgeError>;
    async fn get_session_by_hash(&self, token_hash: &str) -> Result<Option<UserSession>, ForgeError>;
    async fn revoke_session(&self, session_id: &str) -> Result<(), ForgeError>;
    async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<(), ForgeError>;
    async fn list_user_sessions(&self, user_id: &str) -> Result<Vec<UserSession>, ForgeError>;

    // Verification & Reset Tokens
    async fn create_email_token(
        &self,
        token_hash: &str,
        user_id: &str,
        expires_at_nanos: i64,
    ) -> Result<(), ForgeError>;
    async fn consume_email_token(
        &self,
        token_hash: &str,
        now_nanos: i64,
    ) -> Result<Option<String>, ForgeError>;

    async fn create_reset_token(
        &self,
        token_hash: &str,
        user_id: &str,
        expires_at_nanos: i64,
    ) -> Result<(), ForgeError>;
    async fn consume_reset_token(
        &self,
        token_hash: &str,
        now_nanos: i64,
    ) -> Result<Option<String>, ForgeError>;

    // Personal Access Tokens
    async fn create_pat(&self, pat: PersonalAccessToken) -> Result<PersonalAccessToken, ForgeError>;
    async fn get_pat_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<PersonalAccessToken>, ForgeError>;
    async fn update_pat_last_used(
        &self,
        pat_id: &str,
        now_nanos: i64,
    ) -> Result<(), ForgeError>;
    async fn list_user_pats(&self, user_id: &str) -> Result<Vec<PersonalAccessToken>, ForgeError>;
    async fn revoke_pat(&self, pat_id: &str, user_id: &str) -> Result<(), ForgeError>;

    // Repositories
    async fn create_repository(&self, repo: ForgeRepository) -> Result<ForgeRepository, ForgeError>;
    async fn get_repository_by_id(&self, id: &str) -> Result<Option<ForgeRepository>, ForgeError>;
    async fn get_repository_by_owner_name(
        &self,
        owner: &str,
        name: &str,
    ) -> Result<Option<ForgeRepository>, ForgeError>;
    async fn list_repositories_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<ForgeRepository>, ForgeError>;
    async fn list_public_repositories(&self) -> Result<Vec<ForgeRepository>, ForgeError>;
    async fn update_repository_status(
        &self,
        repo_id: &str,
        status: ProvisioningStatus,
        now_nanos: i64,
    ) -> Result<(), ForgeError>;
    async fn delete_repository(&self, repo_id: &str) -> Result<(), ForgeError>;

    // Permissions
    async fn create_permission(
        &self,
        perm: RepositoryPermission,
    ) -> Result<RepositoryPermission, ForgeError>;
    async fn get_permission(
        &self,
        repo_id: &str,
        user_id: &str,
    ) -> Result<Option<PermissionRole>, ForgeError>;
    async fn list_permissions_for_repo(
        &self,
        repo_id: &str,
    ) -> Result<Vec<RepositoryPermission>, ForgeError>;
    async fn delete_permission(&self, repo_id: &str, user_id: &str) -> Result<(), ForgeError>;

    // Provisioning Jobs (Outbox)
    async fn create_provisioning_job(
        &self,
        job: ProvisioningJob,
    ) -> Result<ProvisioningJob, ForgeError>;
    async fn get_next_pending_job(&self) -> Result<Option<ProvisioningJob>, ForgeError>;
    async fn update_job_status(
        &self,
        job_id: &str,
        status: ProvisioningStatus,
        last_error: Option<String>,
        now_nanos: i64,
    ) -> Result<(), ForgeError>;

    // Audit Events
    async fn record_audit_event(&self, event: AuditEvent) -> Result<(), ForgeError>;
}
