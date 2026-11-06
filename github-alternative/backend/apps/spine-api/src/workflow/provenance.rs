use crate::core::types::{PermissionRole, TrustClassification, WorkflowProvenance};
use crate::db::Database;

impl WorkflowProvenance {
    /// Evaluates trust and authorization for a workflow execution.
    ///
    /// Non-negotiable security rules:
    /// 1. If `is_fork` is true (or source_repo != target_repo), always classify as `Untrusted`.
    /// 2. If event starts with `untrusted`, `fork`, or contains untrusted tags, classify as `Untrusted`.
    /// 3. If actor is not an authorized collaborator or owner of target repository, classify as `Untrusted`.
    /// 4. If missing, corrupted, or ambiguous provenance, fail closed to `Untrusted`.
    /// 5. Only if internal repo, non-fork, and actor has confirmed write/admin permissions -> classify as `Trusted`.
    pub async fn evaluate(
        db: &dyn Database,
        target_repo_id: &str,
        source_repo_id: Option<&str>,
        actor_id: &str,
        event_type: &str,
    ) -> Self {
        let is_fork = match source_repo_id {
            Some(s) => s != target_repo_id,
            None => event_type.contains("fork"),
        };

        let repo = match db.get_repository_by_id(target_repo_id).await {
            Ok(Some(r)) => r,
            _ => {
                return Self {
                    target_repo_id: target_repo_id.to_string(),
                    source_repo_id: source_repo_id.map(str::to_string),
                    is_fork,
                    actor_id: actor_id.to_string(),
                    is_actor_authorized: false,
                    trust_classification: TrustClassification::Untrusted,
                    reason: "Target repository not found or inaccessible; failing closed"
                        .to_string(),
                };
            }
        };

        // Determine if actor is authorized (owner or collaborator with write/admin permission)
        let is_actor_authorized = if repo.owner_user_id == actor_id {
            true
        } else if let Ok(Some(role)) = db.get_permission(target_repo_id, actor_id).await {
            role == PermissionRole::Collaborator || role == PermissionRole::Owner
        } else {
            false
        };

        if is_fork {
            return Self {
                target_repo_id: target_repo_id.to_string(),
                source_repo_id: source_repo_id.map(str::to_string),
                is_fork: true,
                actor_id: actor_id.to_string(),
                is_actor_authorized,
                trust_classification: TrustClassification::Untrusted,
                reason: "Workflow originates from a forked repository".to_string(),
            };
        }

        if event_type.starts_with("untrusted") || event_type.contains("fork") {
            return Self {
                target_repo_id: target_repo_id.to_string(),
                source_repo_id: source_repo_id.map(str::to_string),
                is_fork: false,
                actor_id: actor_id.to_string(),
                is_actor_authorized,
                trust_classification: TrustClassification::Untrusted,
                reason: format!("Workflow triggered by untrusted event: {}", event_type),
            };
        }

        if !is_actor_authorized {
            return Self {
                target_repo_id: target_repo_id.to_string(),
                source_repo_id: source_repo_id.map(str::to_string),
                is_fork: false,
                actor_id: actor_id.to_string(),
                is_actor_authorized: false,
                trust_classification: TrustClassification::Untrusted,
                reason: "Actor lacks authorized collaborator or owner status on target repository"
                    .to_string(),
            };
        }

        Self {
            target_repo_id: target_repo_id.to_string(),
            source_repo_id: source_repo_id.map(str::to_string),
            is_fork: false,
            actor_id: actor_id.to_string(),
            is_actor_authorized: true,
            trust_classification: TrustClassification::Trusted,
            reason: "Verified repository event triggered by authorized actor".to_string(),
        }
    }
}
