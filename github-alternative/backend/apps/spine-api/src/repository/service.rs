use crate::core::error::ForgeError;
use crate::core::types::*;
use crate::db::Database;
use crate::permissions::PermissionService;
use spine_cluster::durability::DurabilityPolicy;
use std::sync::Arc;

pub struct RepositoryService {
    db: Arc<dyn Database>,
    permissions: Arc<PermissionService>,
}

impl RepositoryService {
    pub fn new(db: Arc<dyn Database>, permissions: Arc<PermissionService>) -> Self {
        Self { db, permissions }
    }

    pub fn validate_repo_name(name: &str) -> Result<String, ForgeError> {
        let trimmed = name.trim();
        if trimmed.len() < 2 || trimmed.len() > 100 {
            return Err(ForgeError::BadRequest(
                "Repository name must be between 2 and 100 characters".into(),
            ));
        }

        let is_valid = trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
        if !is_valid
            || trimmed.starts_with('.')
            || trimmed.starts_with('-')
            || trimmed.contains("..")
            || trimmed.ends_with(".git")
            || trimmed.ends_with(".spine")
        {
            return Err(ForgeError::BadRequest(
                "Repository name may only contain alphanumeric characters, hyphens, dots, and underscores, cannot contain consecutive dots, and cannot start with '.' or '-'".into(),
            ));
        }

        Ok(trimmed.to_string())
    }

    pub async fn create_repository(
        &self,
        creator: &User,
        name: &str,
        description: Option<String>,
        is_private: bool,
        durability: Option<DurabilityPolicy>,
    ) -> Result<ForgeRepository, ForgeError> {
        let valid_name = Self::validate_repo_name(name)?;
        let normalized = valid_name.to_lowercase();
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let repo_id = format!("repo_{}", uuid::Uuid::new_v4().simple());
        let native_repo_id = format!("{}_{}", creator.username.to_lowercase(), normalized);
        let policy = durability.unwrap_or(DurabilityPolicy::ThreeCopies);

        let repo = ForgeRepository {
            id: repo_id.clone(),
            owner_user_id: creator.id.clone(),
            owner_username: creator.username.clone(),
            owner_org_id: None,
            name: valid_name,
            normalized_name: normalized,
            description: description.unwrap_or_default(),
            is_private,
            default_branch: "main".to_string(),
            native_repo_id,
            durability_policy: policy,
            provisioning_status: ProvisioningStatus::Pending,
            created_at_nanos: now,
            updated_at_nanos: now,
            archived: false,
        };

        // 1. Transactional/atomic insert of repository
        let created_repo = self.db.create_repository(repo).await?;

        // 2. Grant Owner permission
        let perm = RepositoryPermission {
            id: format!("perm_{}", uuid::Uuid::new_v4().simple()),
            repo_id: created_repo.id.clone(),
            user_id: creator.id.clone(),
            role: PermissionRole::Owner,
            created_at_nanos: now,
        };
        let _ = self.db.create_permission(perm).await;

        // 3. Queue durable Provisioning Job (outbox)
        let job = ProvisioningJob {
            id: format!("job_{}", uuid::Uuid::new_v4().simple()),
            repo_id: created_repo.id.clone(),
            status: ProvisioningStatus::Pending,
            attempts: 0,
            last_error: None,
            idempotency_key: format!("prov_{}", created_repo.id),
            created_at_nanos: now,
            updated_at_nanos: now,
        };
        let _ = self.db.create_provisioning_job(job).await;

        // 4. Audit event
        let audit = AuditEvent {
            id: 0,
            user_id: Some(creator.id.clone()),
            action: "repository.create".to_string(),
            resource_type: "repository".to_string(),
            resource_id: created_repo.id.clone(),
            ip_address: None,
            metadata: serde_json::json!({
                "owner": creator.username,
                "name": created_repo.name,
                "is_private": created_repo.is_private,
                "durability": format!("{:?}", created_repo.durability_policy),
            }),
            timestamp_nanos: now,
        };
        let _ = self.db.record_audit_event(audit).await;

        Ok(created_repo)
    }

    pub async fn get_repository(
        &self,
        caller: Option<&User>,
        owner: &str,
        name: &str,
    ) -> Result<ForgeRepository, ForgeError> {
        let repo = self
            .db
            .get_repository_by_owner_name(owner, name)
            .await?
            .ok_or_else(|| ForgeError::NotFound("Repository not found".into()))?;

        // Authorize caller
        self.permissions.check_read_access(caller, &repo).await?;
        Ok(repo)
    }

    pub async fn list_repositories(
        &self,
        caller: Option<&User>,
    ) -> Result<Vec<ForgeRepository>, ForgeError> {
        match caller {
            Some(u) => self.db.list_repositories_for_user(&u.id).await,
            None => self.db.list_public_repositories().await,
        }
    }

    pub async fn delete_repository(
        &self,
        caller: &User,
        owner: &str,
        name: &str,
    ) -> Result<(), ForgeError> {
        let repo = self
            .db
            .get_repository_by_owner_name(owner, name)
            .await?
            .ok_or_else(|| ForgeError::NotFound("Repository not found".into()))?;

        self.permissions
            .check_delete_access(Some(caller), &repo)
            .await?;
        self.db.delete_repository(&repo.id).await?;

        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let audit = AuditEvent {
            id: 0,
            user_id: Some(caller.id.clone()),
            action: "repository.delete".to_string(),
            resource_type: "repository".to_string(),
            resource_id: repo.id.clone(),
            ip_address: None,
            metadata: serde_json::json!({
                "owner": repo.owner_username,
                "name": repo.name,
            }),
            timestamp_nanos: now,
        };
        let _ = self.db.record_audit_event(audit).await;

        Ok(())
    }
}
