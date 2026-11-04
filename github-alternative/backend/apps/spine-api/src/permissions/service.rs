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

    /// Evaluates if caller is an admin or owner of the organization.
    pub async fn check_org_admin(
        &self,
        caller: &User,
        org_id: &str,
    ) -> Result<OrganizationRole, ForgeError> {
        let org = self
            .db
            .get_organization_by_id(org_id)
            .await?
            .ok_or_else(|| ForgeError::NotFound("Organization not found".into()))?;

        if org.owner_user_id == caller.id {
            return Ok(OrganizationRole::Owner);
        }

        if let Some(m) = self
            .db
            .get_organization_membership(org_id, &caller.id)
            .await?
        {
            if m.role == OrganizationRole::Owner || m.role == OrganizationRole::Admin {
                return Ok(m.role);
            }
        }

        if org.is_private {
            Err(ForgeError::NotFound("Organization not found".into()))
        } else {
            Err(ForgeError::Forbidden(
                "Administrative privileges required for this organization".into(),
            ))
        }
    }

    /// Evaluates if caller can view organization metadata.
    pub async fn check_org_read_access(
        &self,
        caller: Option<&User>,
        org: &Organization,
    ) -> Result<Option<OrganizationRole>, ForgeError> {
        if !org.is_private {
            if let Some(user) = caller {
                if user.id == org.owner_user_id {
                    return Ok(Some(OrganizationRole::Owner));
                }
                if let Some(m) = self
                    .db
                    .get_organization_membership(&org.id, &user.id)
                    .await?
                {
                    return Ok(Some(m.role));
                }
            }
            return Ok(None);
        }

        // Private organization:
        let user = caller.ok_or_else(|| ForgeError::NotFound("Organization not found".into()))?;
        if user.id == org.owner_user_id {
            return Ok(Some(OrganizationRole::Owner));
        }
        if let Some(m) = self
            .db
            .get_organization_membership(&org.id, &user.id)
            .await?
        {
            return Ok(Some(m.role));
        }

        Err(ForgeError::NotFound("Organization not found".into()))
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
                if let Some(org_id) = &repo.owner_org_id {
                    if let Ok(role) = self.check_org_admin(user, org_id).await {
                        return Ok(match role {
                            OrganizationRole::Owner => PermissionRole::Owner,
                            _ => PermissionRole::Collaborator,
                        });
                    }
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

        // Check Org admin / membership if repo belongs to an organization
        if let Some(org_id) = &repo.owner_org_id {
            if let Ok(role) = self.check_org_admin(user, org_id).await {
                return Ok(match role {
                    OrganizationRole::Owner => PermissionRole::Owner,
                    _ => PermissionRole::Collaborator,
                });
            }
            // Check team repository permissions in org
            let teams = self
                .db
                .list_teams_for_user_in_org(org_id, &user.id)
                .await
                .unwrap_or_default();
            for t in teams {
                if let Ok(Some(level)) = self
                    .db
                    .get_team_repository_permission(&t.id, &repo.id)
                    .await
                {
                    return Ok(match level {
                        TeamPermissionLevel::Admin => PermissionRole::Owner,
                        TeamPermissionLevel::Maintain | TeamPermissionLevel::Write => {
                            PermissionRole::Collaborator
                        }
                        TeamPermissionLevel::Read => PermissionRole::Reader,
                    });
                }
            }
            // Check base org membership
            if let Ok(Some(_)) = self.db.get_organization_membership(org_id, &user.id).await {
                return Ok(PermissionRole::Reader);
            }
        }

        // Check direct collaborator permissions
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
        let user = caller
            .ok_or_else(|| ForgeError::Unauthorized("Authentication required to write".into()))?;
        if repo.archived {
            return Err(ForgeError::NotFound("Repository not found".into()));
        }

        if user.id == repo.owner_user_id {
            return Ok(PermissionRole::Owner);
        }

        if let Some(org_id) = &repo.owner_org_id {
            if let Ok(role) = self.check_org_admin(user, org_id).await {
                return Ok(match role {
                    OrganizationRole::Owner => PermissionRole::Owner,
                    _ => PermissionRole::Collaborator,
                });
            }
            let teams = self
                .db
                .list_teams_for_user_in_org(org_id, &user.id)
                .await
                .unwrap_or_default();
            for t in teams {
                if let Ok(Some(level)) = self
                    .db
                    .get_team_repository_permission(&t.id, &repo.id)
                    .await
                {
                    match level {
                        TeamPermissionLevel::Admin
                        | TeamPermissionLevel::Maintain
                        | TeamPermissionLevel::Write => {
                            return Ok(PermissionRole::Collaborator);
                        }
                        TeamPermissionLevel::Read => {}
                    }
                }
            }
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
        let user =
            caller.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
        if user.id == repo.owner_user_id {
            return Ok(());
        }

        if let Some(org_id) = &repo.owner_org_id {
            if let Ok(_) = self.check_org_admin(user, org_id).await {
                return Ok(());
            }
            let teams = self
                .db
                .list_teams_for_user_in_org(org_id, &user.id)
                .await
                .unwrap_or_default();
            for t in teams {
                if let Ok(Some(TeamPermissionLevel::Admin)) = self
                    .db
                    .get_team_repository_permission(&t.id, &repo.id)
                    .await
                {
                    return Ok(());
                }
            }
        }

        if repo.is_private {
            Err(ForgeError::NotFound("Repository not found".into()))
        } else {
            Err(ForgeError::Forbidden(
                "Administrative access requires repository ownership or admin permissions".into(),
            ))
        }
    }

    /// Evaluates if user can delete repository.
    pub async fn check_delete_access(
        &self,
        caller: Option<&User>,
        repo: &ForgeRepository,
    ) -> Result<(), ForgeError> {
        let user =
            caller.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
        if user.id == repo.owner_user_id {
            return Ok(());
        }

        if let Some(org_id) = &repo.owner_org_id {
            if let Ok(OrganizationRole::Owner) = self.check_org_admin(user, org_id).await {
                return Ok(());
            }
        }

        if repo.is_private {
            Err(ForgeError::NotFound("Repository not found".into()))
        } else {
            Err(ForgeError::Forbidden(
                "Only the repository owner or organization owner may delete this repository".into(),
            ))
        }
    }

    /// Evaluates whether a pull request can be merged into the target branch given branch protection rules.
    pub async fn check_merge_allowed(
        &self,
        caller: &User,
        repo: &ForgeRepository,
        target_branch: &str,
        pr: &PullRequest,
    ) -> Result<(), ForgeError> {
        // First ensure user has write access
        self.check_write_access(Some(caller), repo).await?;

        let rule = match self
            .db
            .get_branch_protection(&repo.id, target_branch)
            .await?
        {
            Some(r) => r,
            None => return Ok(()), // No protection rules configured
        };

        if rule.restrict_merges_to_admins {
            self.check_admin_access(Some(caller), repo).await.map_err(|_| {
                ForgeError::Forbidden("Branch protection: merges to this branch are restricted to repository administrators".into())
            })?;
        }

        let reviews = self.db.list_pr_reviews(&pr.id).await?;
        let mut approval_count = 0;
        let mut has_changes_requested = false;

        // Take the latest non-dismissed review per reviewer
        let mut latest_by_reviewer = std::collections::HashMap::new();
        for rev in reviews {
            if rev.state != "DISMISSED" {
                latest_by_reviewer.insert(rev.reviewer_id.clone(), rev);
            }
        }

        for (_, rev) in latest_by_reviewer {
            if rev.state == "APPROVED" {
                if rule.prevent_self_approval && rev.reviewer_id == pr.author_id {
                    continue; // Skip self-approval
                }
                approval_count += 1;
            } else if rev.state == "CHANGES_REQUESTED" {
                has_changes_requested = true;
            }
        }

        if approval_count < rule.required_approvals {
            return Err(ForgeError::UnprocessableEntity(format!(
                "Branch protection: {} approved review(s) required (currently has {})",
                rule.required_approvals, approval_count
            )));
        }

        if rule.require_changes_addressed && has_changes_requested {
            return Err(ForgeError::UnprocessableEntity(
                "Branch protection: cannot merge while there are outstanding change requests"
                    .into(),
            ));
        }

        if rule.require_changes_addressed {
            let comments = self.db.list_pr_review_comments(&pr.id).await?;
            let unresolved = comments.iter().any(|c| !c.is_resolved);
            if unresolved {
                return Err(ForgeError::UnprocessableEntity(
                    "Branch protection: all inline review comments must be resolved before merging"
                        .into(),
                ));
            }
        }

        if !rule.required_status_checks.is_empty() {
            let checks = self
                .db
                .list_workflow_check_results(&repo.id, &pr.head_commit_id)
                .await?;
            for req_check in &rule.required_status_checks {
                let check_opt = checks.iter().find(|c| c.check_name == *req_check);
                match check_opt {
                    Some(c) if c.status == "SUCCESS" => {}
                    Some(c) => {
                        return Err(ForgeError::UnprocessableEntity(format!(
                            "Branch protection: required check '{}' has status '{}', must be 'SUCCESS' before merging",
                            req_check, c.status
                        )));
                    }
                    None => {
                        return Err(ForgeError::UnprocessableEntity(format!(
                            "Branch protection: required check '{}' has not passed for commit {}",
                            req_check, pr.head_commit_id
                        )));
                    }
                }
            }
        }

        Ok(())
    }

    pub async fn can_read_repo(
        &self,
        user_id: Option<&str>,
        repo_id: &str,
    ) -> Result<bool, ForgeError> {
        let repo = match self.db.get_repository_by_id(repo_id).await? {
            Some(r) => r,
            None => return Ok(false),
        };
        let caller = if let Some(uid) = user_id {
            self.db.get_user_by_id(uid).await?
        } else {
            None
        };
        Ok(self.check_read_access(caller.as_ref(), &repo).await.is_ok())
    }

    pub async fn require_repo_read(
        &self,
        user_id: Option<&str>,
        repo_id: &str,
    ) -> Result<(), ForgeError> {
        let repo = self
            .db
            .get_repository_by_id(repo_id)
            .await?
            .ok_or_else(|| ForgeError::NotFound("Repository not found".into()))?;
        let caller = if let Some(uid) = user_id {
            self.db.get_user_by_id(uid).await?
        } else {
            None
        };
        self.check_read_access(caller.as_ref(), &repo).await?;
        Ok(())
    }

    pub async fn require_repo_write(&self, user_id: &str, repo_id: &str) -> Result<(), ForgeError> {
        let repo = self
            .db
            .get_repository_by_id(repo_id)
            .await?
            .ok_or_else(|| ForgeError::NotFound("Repository not found".into()))?;
        let caller = self
            .db
            .get_user_by_id(user_id)
            .await?
            .ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
        self.check_write_access(Some(&caller), &repo).await?;
        Ok(())
    }

    pub async fn can_admin_repo(&self, user_id: &str, repo_id: &str) -> Result<bool, ForgeError> {
        let repo = match self.db.get_repository_by_id(repo_id).await? {
            Some(r) => r,
            None => return Ok(false),
        };
        let caller = match self.db.get_user_by_id(user_id).await? {
            Some(u) => u,
            None => return Ok(false),
        };
        Ok(self.check_admin_access(Some(&caller), &repo).await.is_ok())
    }
}
