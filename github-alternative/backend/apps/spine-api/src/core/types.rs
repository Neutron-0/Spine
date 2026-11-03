use serde::{Deserialize, Serialize};
use spine_cluster::durability::DurabilityPolicy;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub username: String,
    pub normalized_username: String,
    pub email: String,
    pub normalized_email: String,
    pub display_name: String,
    pub password_hash: String,
    pub account_status: String,
    pub avatar_url: Option<String>,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
    pub last_login_nanos: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserDto {
    pub id: String,
    pub username: String,
    pub email: String,
    pub display_name: String,
    pub account_status: String,
    pub avatar_url: Option<String>,
    pub created_at_nanos: i64,
}

impl From<&User> for UserDto {
    fn from(u: &User) -> Self {
        Self {
            id: u.id.clone(),
            username: u.username.clone(),
            email: u.email.clone(),
            display_name: u.display_name.clone(),
            account_status: u.account_status.clone(),
            avatar_url: u.avatar_url.clone(),
            created_at_nanos: u.created_at_nanos,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserSession {
    pub id: String,
    pub user_id: String,
    pub session_token_hash: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub created_at_nanos: i64,
    pub expires_at_nanos: i64,
    pub revoked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionDto {
    pub id: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub created_at_nanos: i64,
    pub expires_at_nanos: i64,
    pub is_active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalAccessToken {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub token_hash: String,
    pub scopes: Vec<String>,
    pub expires_at_nanos: i64,
    pub created_at_nanos: i64,
    pub last_used_at_nanos: Option<i64>,
    pub revoked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatDto {
    pub id: String,
    pub name: String,
    pub scopes: Vec<String>,
    pub expires_at_nanos: i64,
    pub created_at_nanos: i64,
    pub last_used_at_nanos: Option<i64>,
    pub revoked: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProvisioningStatus {
    Pending,
    Provisioning,
    Ready,
    Failed,
    Deleting,
    Tombstoned,
}

impl ProvisioningStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Provisioning => "PROVISIONING",
            Self::Ready => "READY",
            Self::Failed => "FAILED",
            Self::Deleting => "DELETING",
            Self::Tombstoned => "TOMBSTONED",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "PROVISIONING" => Self::Provisioning,
            "READY" => Self::Ready,
            "FAILED" => Self::Failed,
            "DELETING" => Self::Deleting,
            "TOMBSTONED" => Self::Tombstoned,
            _ => Self::Pending,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgeRepository {
    pub id: String,
    pub owner_user_id: String,
    pub owner_username: String,
    pub name: String,
    pub normalized_name: String,
    pub description: String,
    pub is_private: bool,
    pub default_branch: String,
    pub native_repo_id: String,
    pub durability_policy: DurabilityPolicy,
    pub provisioning_status: ProvisioningStatus,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
    pub archived: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryDto {
    pub id: String,
    pub owner: String,
    pub name: String,
    pub description: String,
    pub is_private: bool,
    pub default_branch: String,
    pub native_repo_id: String,
    pub durability_policy: DurabilityPolicy,
    pub provisioning_status: String,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
}

impl From<&ForgeRepository> for RepositoryDto {
    fn from(r: &ForgeRepository) -> Self {
        Self {
            id: r.id.clone(),
            owner: r.owner_username.clone(),
            name: r.name.clone(),
            description: r.description.clone(),
            is_private: r.is_private,
            default_branch: r.default_branch.clone(),
            native_repo_id: r.native_repo_id.clone(),
            durability_policy: r.durability_policy,
            provisioning_status: r.provisioning_status.as_str().to_string(),
            created_at_nanos: r.created_at_nanos,
            updated_at_nanos: r.updated_at_nanos,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionRole {
    Reader,
    Collaborator,
    Owner,
}

impl PermissionRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Reader => "READER",
            Self::Collaborator => "COLLABORATOR",
            Self::Owner => "OWNER",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "OWNER" => Self::Owner,
            "READER" => Self::Reader,
            _ => Self::Collaborator,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryPermission {
    pub id: String,
    pub repo_id: String,
    pub user_id: String,
    pub role: PermissionRole,
    pub created_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvisioningJob {
    pub id: String,
    pub repo_id: String,
    pub status: ProvisioningStatus,
    pub attempts: u32,
    pub last_error: Option<String>,
    pub idempotency_key: String,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: i64,
    pub user_id: Option<String>,
    pub action: String,
    pub resource_type: String,
    pub resource_id: String,
    pub ip_address: Option<String>,
    pub metadata: serde_json::Value,
    pub timestamp_nanos: i64,
}
