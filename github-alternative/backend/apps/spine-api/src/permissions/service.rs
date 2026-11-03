use crate::core::error::ForgeError;
use crate::core::types::*;
use crate::db::Database;
use std::sync::Arc;

pub struct PermissionService {
    db: Arc<dyn Database>,
}

impl PermissionService {
    pub fn new(db: Arc<dyn Database>) -> Self {
        Self { db }
    }

    /// Evaluates if user can see repository metadata and read contents.
    /// If private and user not authorized, returns `ForgeError::NotFound` to prevent information disclosure.
    pub async fn check_read_access(
        &self,
        caller: Option<&User>,
        repo: &ForgeRepository,
    ) -> Result<PermissionRole, ForgeError> {
        if repo.archived {
            return Err(ForgeError::NotFound("Repository not found".into()));
        }

        // If repo is public, anonymous or any user has at least Reader access
        if !repo.is_private {
            if let Some(user) = caller {
                if user.id == repo.owner_user_id {
                    return Ok(PermissionRole::Owner);
                }
                if let Ok(Some(role)) = self.db.get_permission(&repo.id, &user.id).await {
                    return Ok(role);
                }
            }
            return Ok(PermissionRole::Reader);
        }

        // Repo is private: caller must be authenticated
        let user = match caller {
            Some(u) => u,
            None => {
                // Must return NotFound to prevent existence discovery of private repositories
                return Err(ForgeError::NotFound("Repository not found".into()));
            }
        };

        if user.id == repo.owner_user_id {
            return Ok(PermissionRole::Owner);
        }

        if let Ok(Some(role)) = self.db.get_permission(&repo.id, &user.id).await {
            return Ok(role);
        }

        // Authenticated user with no permission to private repo
        Err(ForgeError::NotFound("Repository not found".into()))
    }

    /// Evaluates if user can push to repository.
    pub async fn check_write_access(
        &self,
        caller: Option<&User>,
        repo: &ForgeRepository,
    ) -> Result<PermissionRole, ForgeError> {
        let user = caller.ok_or_else(|| ForgeError::Unauthorized("Authentication required to write".into()))?;
        if repo.archived {
            return Err(ForgeError::NotFound("Repository not found".into()));
        }

        if user.id == repo.owner_user_id {
            return Ok(PermissionRole::Owner);
        }

        if let Ok(Some(role)) = self.db.get_permission(&repo.id, &user.id).await {
            if role == PermissionRole::Collaborator || role == PermissionRole::Owner {
                return Ok(role);
            }
        }

        if repo.is_private {
            Err(ForgeError::NotFound("Repository not found".into()))
        } else {
            Err(ForgeError::Forbidden(
                "Write access denied: you are not a collaborator on this repository".into(),
            ))
        }
    }

    /// Evaluates if user can admin repository settings or collaborators.
    pub async fn check_admin_access(
        &self,
        caller: Option<&User>,
        repo: &ForgeRepository,
    ) -> Result<(), ForgeError> {
        let user = caller.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
        if user.id == repo.owner_user_id {
            return Ok(());
        }

        if repo.is_private {
            Err(ForgeError::NotFound("Repository not found".into()))
        } else {
            Err(ForgeError::Forbidden(
                "Administrative access requires repository ownership".into(),
            ))
        }
    }

    /// Evaluates if user can delete repository.
    pub async fn check_delete_access(
        &self,
        caller: Option<&User>,
        repo: &ForgeRepository,
    ) -> Result<(), ForgeError> {
        let user = caller.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
        if user.id == repo.owner_user_id {
            return Ok(());
        }

        if repo.is_private {
            Err(ForgeError::NotFound("Repository not found".into()))
        } else {
            Err(ForgeError::Forbidden(
                "Only the repository owner may delete this repository".into(),
            ))
        }
    }
}
