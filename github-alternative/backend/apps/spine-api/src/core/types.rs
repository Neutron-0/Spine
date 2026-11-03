use serde::{Deserialize, Serialize};
use spine_cluster::durability::DurabilityPolicy;

// -----------------------------------------------------------------------------
// Identity & Authentication Models
// -----------------------------------------------------------------------------

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

// -----------------------------------------------------------------------------
// Repository & Storage Models
// -----------------------------------------------------------------------------

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
    pub owner_org_id: Option<String>,
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
    pub owner_org_id: Option<String>,
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
            owner_org_id: r.owner_org_id.clone(),
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

// -----------------------------------------------------------------------------
// Organization & Team Models (Batch 09)
// -----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrganizationRole {
    Owner,
    Admin,
    Member,
}

impl OrganizationRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Owner => "OWNER",
            Self::Admin => "ADMIN",
            Self::Member => "MEMBER",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "OWNER" => Self::Owner,
            "ADMIN" => Self::Admin,
            _ => Self::Member,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Organization {
    pub id: String,
    pub name: String,
    pub normalized_name: String,
    pub display_name: String,
    pub description: String,
    pub avatar_url: Option<String>,
    pub owner_user_id: String,
    pub is_private: bool,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrganizationDto {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub avatar_url: Option<String>,
    pub owner_user_id: String,
    pub is_private: bool,
    pub created_at_nanos: i64,
}

impl From<&Organization> for OrganizationDto {
    fn from(o: &Organization) -> Self {
        Self {
            id: o.id.clone(),
            name: o.name.clone(),
            display_name: o.display_name.clone(),
            description: o.description.clone(),
            avatar_url: o.avatar_url.clone(),
            owner_user_id: o.owner_user_id.clone(),
            is_private: o.is_private,
            created_at_nanos: o.created_at_nanos,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrganizationMembership {
    pub id: String,
    pub org_id: String,
    pub user_id: String,
    pub role: OrganizationRole,
    pub created_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrganizationMemberDto {
    pub user: UserDto,
    pub role: OrganizationRole,
    pub created_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrganizationInvitation {
    pub id: String,
    pub org_id: String,
    pub inviter_user_id: String,
    pub invitee_email: String,
    pub role: OrganizationRole,
    pub token_hash: String,
    pub status: String,
    pub expires_at_nanos: i64,
    pub created_at_nanos: i64,
    pub responded_at_nanos: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrganizationInvitationDto {
    pub id: String,
    pub org_id: String,
    pub inviter_user_id: String,
    pub invitee_email: String,
    pub role: OrganizationRole,
    pub status: String,
    pub expires_at_nanos: i64,
    pub created_at_nanos: i64,
}

impl From<&OrganizationInvitation> for OrganizationInvitationDto {
    fn from(i: &OrganizationInvitation) -> Self {
        Self {
            id: i.id.clone(),
            org_id: i.org_id.clone(),
            inviter_user_id: i.inviter_user_id.clone(),
            invitee_email: i.invitee_email.clone(),
            role: i.role,
            status: i.status.clone(),
            expires_at_nanos: i.expires_at_nanos,
            created_at_nanos: i.created_at_nanos,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Team {
    pub id: String,
    pub org_id: String,
    pub name: String,
    pub normalized_slug: String,
    pub description: String,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamDto {
    pub id: String,
    pub org_id: String,
    pub name: String,
    pub slug: String,
    pub description: String,
    pub created_at_nanos: i64,
}

impl From<&Team> for TeamDto {
    fn from(t: &Team) -> Self {
        Self {
            id: t.id.clone(),
            org_id: t.org_id.clone(),
            name: t.name.clone(),
            slug: t.normalized_slug.clone(),
            description: t.description.clone(),
            created_at_nanos: t.created_at_nanos,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamMembership {
    pub id: String,
    pub team_id: String,
    pub user_id: String,
    pub is_maintainer: bool,
    pub created_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamMemberDto {
    pub user: UserDto,
    pub is_maintainer: bool,
    pub created_at_nanos: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TeamPermissionLevel {
    Read,
    Write,
    Maintain,
    Admin,
}

impl TeamPermissionLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Read => "READ",
            Self::Write => "WRITE",
            Self::Maintain => "MAINTAIN",
            Self::Admin => "ADMIN",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s.to_uppercase().as_str() {
            "WRITE" => Self::Write,
            "MAINTAIN" => Self::Maintain,
            "ADMIN" => Self::Admin,
            _ => Self::Read,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamRepositoryPermission {
    pub id: String,
    pub team_id: String,
    pub repo_id: String,
    pub permission_level: TeamPermissionLevel,
    pub created_at_nanos: i64,
}

// -----------------------------------------------------------------------------
// Issues, Labels, Milestones (Batch 09)
// -----------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Milestone {
    pub id: String,
    pub repo_id: String,
    pub title: String,
    pub description: String,
    pub state: String,
    pub due_date_nanos: Option<i64>,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
    pub closed_at_nanos: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MilestoneDto {
    pub id: String,
    pub title: String,
    pub description: String,
    pub state: String,
    pub due_date_nanos: Option<i64>,
    pub created_at_nanos: i64,
}

impl From<&Milestone> for MilestoneDto {
    fn from(m: &Milestone) -> Self {
        Self {
            id: m.id.clone(),
            title: m.title.clone(),
            description: m.description.clone(),
            state: m.state.clone(),
            due_date_nanos: m.due_date_nanos,
            created_at_nanos: m.created_at_nanos,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryLabel {
    pub id: String,
    pub repo_id: String,
    pub name: String,
    pub color: String,
    pub description: String,
    pub created_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelDto {
    pub id: String,
    pub name: String,
    pub color: String,
    pub description: String,
}

impl From<&RepositoryLabel> for LabelDto {
    fn from(l: &RepositoryLabel) -> Self {
        Self {
            id: l.id.clone(),
            name: l.name.clone(),
            color: l.color.clone(),
            description: l.description.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    pub id: String,
    pub repo_id: String,
    pub issue_number: i64,
    pub author_id: String,
    pub title: String,
    pub body: String,
    pub status: String,
    pub milestone_id: Option<String>,
    pub closed_by_user_id: Option<String>,
    pub closed_at_nanos: Option<i64>,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueDto {
    pub id: String,
    pub repo_id: String,
    pub issue_number: i64,
    pub author: UserDto,
    pub title: String,
    pub body: String,
    pub status: String,
    pub milestone: Option<MilestoneDto>,
    pub assignees: Vec<UserDto>,
    pub labels: Vec<LabelDto>,
    pub comments_count: usize,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
    pub closed_at_nanos: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueComment {
    pub id: String,
    pub issue_id: String,
    pub author_id: String,
    pub body: String,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueCommentDto {
    pub id: String,
    pub author: UserDto,
    pub body: String,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
}

// -----------------------------------------------------------------------------
// Pull Requests, Reviews, Inline Comments (Batch 09)
// -----------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequest {
    pub id: String,
    pub repo_id: String,
    pub pr_number: i64,
    pub author_id: String,
    pub title: String,
    pub body: String,
    pub base_branch: String,
    pub head_branch: String,
    pub base_commit_id: String,
    pub head_commit_id: String,
    pub state: String,
    pub is_draft: bool,
    pub merged_by_user_id: Option<String>,
    pub merge_commit_id: Option<String>,
    pub merge_strategy: Option<String>,
    pub closed_at_nanos: Option<i64>,
    pub merged_at_nanos: Option<i64>,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestDto {
    pub id: String,
    pub repo_id: String,
    pub pr_number: i64,
    pub author: UserDto,
    pub title: String,
    pub body: String,
    pub base_branch: String,
    pub head_branch: String,
    pub base_commit_id: String,
    pub head_commit_id: String,
    pub state: String,
    pub is_draft: bool,
    pub merge_commit_id: Option<String>,
    pub mergeable: Option<bool>,
    pub requested_reviewers: Vec<UserDto>,
    pub reviews_count: usize,
    pub comments_count: usize,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
    pub closed_at_nanos: Option<i64>,
    pub merged_at_nanos: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestReview {
    pub id: String,
    pub pr_id: String,
    pub reviewer_id: String,
    pub commit_id: String,
    pub state: String,
    pub body: String,
    pub dismissed_by_user_id: Option<String>,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestReviewDto {
    pub id: String,
    pub reviewer: UserDto,
    pub commit_id: String,
    pub state: String,
    pub body: String,
    pub created_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestReviewComment {
    pub id: String,
    pub pr_id: String,
    pub review_id: Option<String>,
    pub author_id: String,
    pub file_path: String,
    pub commit_id: String,
    pub diff_side: String,
    pub line_number: i32,
    pub body: String,
    pub is_resolved: bool,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestReviewCommentDto {
    pub id: String,
    pub author: UserDto,
    pub file_path: String,
    pub commit_id: String,
    pub diff_side: String,
    pub line_number: i32,
    pub body: String,
    pub is_resolved: bool,
    pub is_outdated: bool,
    pub created_at_nanos: i64,
}

// -----------------------------------------------------------------------------
// Branch Protection (Batch 09)
// -----------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchProtectionRule {
    pub id: String,
    pub repo_id: String,
    pub branch_pattern: String,
    pub require_pull_request: bool,
    pub required_approvals: i32,
    pub prevent_self_approval: bool,
    pub require_changes_addressed: bool,
    pub dismiss_stale_reviews_on_push: bool,
    pub restrict_merges_to_admins: bool,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
}

// -----------------------------------------------------------------------------
// Notifications & Audit Events (Batch 09)
// -----------------------------------------------------------------------------

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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserNotification {
    pub id: String,
    pub user_id: String,
    pub reason: String,
    pub resource_type: String,
    pub resource_id: String,
    pub title: String,
    pub message: String,
    pub is_read: bool,
    pub created_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationDto {
    pub id: String,
    pub reason: String,
    pub resource_type: String,
    pub resource_id: String,
    pub title: String,
    pub message: String,
    pub is_read: bool,
    pub created_at_nanos: i64,
}

impl From<&UserNotification> for NotificationDto {
    fn from(n: &UserNotification) -> Self {
        Self {
            id: n.id.clone(),
            reason: n.reason.clone(),
            resource_type: n.resource_type.clone(),
            resource_id: n.resource_id.clone(),
            title: n.title.clone(),
            message: n.message.clone(),
            is_read: n.is_read,
            created_at_nanos: n.created_at_nanos,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryActivityEvent {
    pub id: String,
    pub repo_id: String,
    pub actor_id: Option<String>,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at_nanos: i64,
}

// -----------------------------------------------------------------------------
// Diff & Mergeability Models (Batch 09)
// -----------------------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiffFileDto {
    pub path: String,
    pub old_path: Option<String>,
    pub change_type: String,
    pub is_binary: bool,
    pub patch: String,
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PullRequestDiffDto {
    pub base_commit_id: String,
    pub head_commit_id: String,
    pub merge_base_commit_id: String,
    pub files: Vec<DiffFileDto>,
    pub total_additions: usize,
    pub total_deletions: usize,
    pub commits_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeabilityReport {
    pub mergeable: bool,
    pub status: String,
    pub conflict_files: Vec<String>,
    pub base_commit_id: String,
    pub head_commit_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeResultDto {
    pub success: bool,
    pub merge_commit_id: Option<String>,
    pub message: String,
}
