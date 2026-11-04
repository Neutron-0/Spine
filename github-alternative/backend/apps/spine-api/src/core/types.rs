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
    #[serde(default)]
    pub required_status_checks: Vec<String>,
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

// -----------------------------------------------------------------------------
// Batch 11: Artifacts, Releases & Packages
// -----------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactQuota {
    pub id: String,
    pub owner_type: String, // "user" or "organization"
    pub owner_id: String,
    pub logical_quota_bytes: i64,
    pub used_logical_bytes: i64,
    pub used_physical_bytes: i64,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactUploadSession {
    pub id: String,
    pub artifact_id: String,
    pub owner_type: String,
    pub owner_id: String,
    pub state: String,
    pub total_size: i64,
    pub chunk_size: i32,
    pub expected_chunks: i32,
    pub durability_policy: String,
    pub created_at_nanos: i64,
    pub expires_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredArtifactManifest {
    pub id: String,
    pub artifact_id: String,
    pub whole_object_hash: String,
    pub total_size: i64,
    pub chunk_size: i32,
    pub chunk_count: i32,
    pub content_type: Option<String>,
    pub manifest_json: String,
    pub created_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateReleaseRequest {
    pub tag_name: String,
    pub target_commit_id: String,
    pub title: String,
    pub body: String,
    pub is_draft: Option<bool>,
    pub is_prerelease: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateReleaseRequest {
    pub title: Option<String>,
    pub body: Option<String>,
    pub is_draft: Option<bool>,
    pub is_prerelease: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub id: String,
    pub repo_id: String,
    pub tag_name: String,
    pub target_commit_id: String,
    pub title: String,
    pub body: String,
    pub is_draft: bool,
    pub is_prerelease: bool,
    pub author_id: String,
    pub created_at_nanos: i64,
    pub published_at_nanos: Option<i64>,
    pub updated_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseDto {
    pub id: String,
    pub repo_id: String,
    pub tag_name: String,
    pub target_commit_id: String,
    pub title: String,
    pub body: String,
    pub is_draft: bool,
    pub is_prerelease: bool,
    pub author: UserDto,
    pub assets: Vec<ReleaseAssetDto>,
    pub created_at_nanos: i64,
    pub published_at_nanos: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseAsset {
    pub id: String,
    pub release_id: String,
    pub name: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub manifest_id: String,
    pub whole_hash: String,
    pub downloads_count: i32,
    pub created_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseAssetDto {
    pub id: String,
    pub release_id: String,
    pub name: String,
    pub content_type: String,
    pub size_bytes: i64,
    pub manifest_id: String,
    pub whole_hash: String,
    pub downloads_count: i32,
    pub created_at_nanos: i64,
}

impl From<&ReleaseAsset> for ReleaseAssetDto {
    fn from(a: &ReleaseAsset) -> Self {
        Self {
            id: a.id.clone(),
            release_id: a.release_id.clone(),
            name: a.name.clone(),
            content_type: a.content_type.clone(),
            size_bytes: a.size_bytes,
            manifest_id: a.manifest_id.clone(),
            whole_hash: a.whole_hash.clone(),
            downloads_count: a.downloads_count,
            created_at_nanos: a.created_at_nanos,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactNamespace {
    pub id: String,
    pub name: String,
    pub owner_type: String,
    pub owner_id: String,
    pub is_private: bool,
    pub description: String,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactNamespaceDto {
    pub id: String,
    pub name: String,
    pub owner_type: String,
    pub owner_id: String,
    pub is_private: bool,
    pub description: String,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
}

impl From<&ArtifactNamespace> for ArtifactNamespaceDto {
    fn from(n: &ArtifactNamespace) -> Self {
        Self {
            id: n.id.clone(),
            name: n.name.clone(),
            owner_type: n.owner_type.clone(),
            owner_id: n.owner_id.clone(),
            is_private: n.is_private,
            description: n.description.clone(),
            created_at_nanos: n.created_at_nanos,
            updated_at_nanos: n.updated_at_nanos,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactVersion {
    pub id: String,
    pub namespace_id: String,
    pub artifact_name: String,
    pub version: String,
    pub manifest_id: String,
    pub whole_hash: String,
    pub size_bytes: i64,
    pub created_by_user_id: String,
    pub created_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactVersionDto {
    pub id: String,
    pub namespace_id: String,
    pub artifact_name: String,
    pub version: String,
    pub manifest_id: String,
    pub whole_hash: String,
    pub size_bytes: i64,
    pub created_by_user_id: String,
    pub created_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Package {
    pub id: String,
    pub ecosystem: String,
    pub name: String,
    pub owner_type: String,
    pub owner_id: String,
    pub is_private: bool,
    pub description: String,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageDto {
    pub id: String,
    pub ecosystem: String,
    pub name: String,
    pub owner_type: String,
    pub owner_id: String,
    pub is_private: bool,
    pub description: String,
    pub versions: Vec<PackageVersionDto>,
    pub versions_count: usize,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageVersion {
    pub id: String,
    pub package_id: String,
    pub version: String,
    pub manifest_id: String,
    pub cksum: String,
    pub size_bytes: i64,
    pub metadata_json: String,
    pub is_yanked: bool,
    pub created_by_user_id: String,
    pub created_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageVersionDto {
    pub id: String,
    pub package_id: String,
    pub version: String,
    pub manifest_id: String,
    pub cksum: String,
    pub size_bytes: i64,
    pub metadata_json: String,
    pub is_yanked: bool,
    pub created_by_user_id: String,
    pub created_at_nanos: i64,
}

// -----------------------------------------------------------------------------
// Webhooks, Workflows, Runners, and CI/CD (Batch 12)
// -----------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateWebhookSubscriptionRequest {
    pub owner_type: String, // "REPO" or "ORG"
    pub owner_id: String,
    pub target_url: String,
    pub secret: Option<String>,
    pub events: Vec<String>,
    pub is_active: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookSubscription {
    pub id: String,
    pub owner_type: String, // "REPO" or "ORG"
    pub owner_id: String,
    pub target_url: String,
    pub secret_hash: String,
    pub secret_ciphertext: String,
    pub events: String,
    pub is_active: bool,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookSubscriptionDto {
    pub id: String,
    pub owner_type: String,
    pub owner_id: String,
    pub target_url: String,
    pub events: Vec<String>,
    pub is_active: bool,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookOutboxItem {
    pub id: String,
    pub event_type: String,
    pub owner_type: String,
    pub owner_id: String,
    pub payload: serde_json::Value,
    pub attempts: i32,
    pub max_attempts: i32,
    pub status: String, // "PENDING", "PROCESSING", "DELIVERED", "FAILED"
    pub last_error: Option<String>,
    pub created_at_nanos: i64,
    pub next_attempt_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookDelivery {
    pub id: String,
    pub subscription_id: String,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub response_status: Option<i32>,
    pub response_body: Option<String>,
    pub execution_duration_ms: i64,
    pub attempt: i32,
    pub status: String,
    pub delivered_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    pub id: String,
    pub repo_id: String,
    pub path: String,
    pub name: String,
    pub yaml_content: String,
    pub digest: String,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRun {
    pub id: String,
    pub repo_id: String,
    pub workflow_id: String,
    pub workflow_name: String,
    pub commit_oid: String,
    pub ref_name: String,
    pub trigger_event: String,
    pub actor_id: String,
    pub status: String, // "QUEUED", "RUNNING", "SUCCEEDED", "FAILED", "CANCELLED"
    pub conclusion: Option<String>,
    pub definition_yaml: String,
    pub definition_digest: String,
    pub created_at_nanos: i64,
    pub started_at_nanos: Option<i64>,
    pub completed_at_nanos: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowJob {
    pub id: String,
    pub run_id: String,
    pub repo_id: String,
    pub job_key: String,
    pub name: String,
    pub status: String, // "QUEUED", "WAITING_FOR_RUNNER", "ASSIGNED", "RUNNING", "SUCCEEDED", "FAILED", "CANCELLED", "TIMED_OUT", "LOST"
    pub runner_id: Option<String>,
    pub runs_on: Vec<String>,
    pub needs: Vec<String>,
    pub attempt: i32,
    pub fencing_token: i64,
    pub lease_expires_at_nanos: Option<i64>,
    pub exit_code: Option<i32>,
    pub created_at_nanos: i64,
    pub started_at_nanos: Option<i64>,
    pub completed_at_nanos: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub id: String,
    pub job_id: String,
    pub step_index: i32,
    pub name: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub started_at_nanos: Option<i64>,
    pub completed_at_nanos: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowLogChunk {
    pub id: String,
    pub job_id: String,
    pub step_index: i32,
    pub stream: String,
    pub content: String,
    pub timestamp_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerRegistration {
    pub id: String,
    pub name: String,
    pub hostname: String,
    pub token_hash: String,
    pub os: String,
    pub arch: String,
    pub labels: Vec<String>,
    pub max_concurrency: i32,
    pub active_jobs: i32,
    pub status: String, // "ACTIVE", "DRAINED", "REVOKED"
    pub last_heartbeat_at_nanos: i64,
    pub created_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowSecret {
    pub id: String,
    pub owner_type: String,
    pub owner_id: String,
    pub name: String,
    pub ciphertext: String,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowCheckResult {
    pub id: String,
    pub repo_id: String,
    pub commit_oid: String,
    pub check_name: String,
    pub status: String, // "PENDING", "SUCCESS", "FAILURE", "ERROR"
    pub run_id: String,
    pub updated_at_nanos: i64,
}

// -----------------------------------------------------------------------------
// Search Models (Batch 13)
// -----------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchResultType {
    Repository,
    Code,
    Issue,
    PullRequest,
    Discussion,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    pub result_type: SearchResultType,
    pub repo_id: String,
    pub repo_name: String,
    pub title: String,
    pub snippet: String,
    pub file_path: Option<String>,
    pub line_number: Option<usize>,
    pub url: String,
    pub score: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchResponse {
    pub query: String,
    pub total: usize,
    pub hits: Vec<SearchHit>,
}

// -----------------------------------------------------------------------------
// Discussions Models (Batch 13)
// -----------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscussionCategory {
    pub id: String,
    pub repo_id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub emoji: String,
    pub is_answerable: bool,
    pub created_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Discussion {
    pub id: String,
    pub repo_id: String,
    pub category_id: String,
    pub number: i32,
    pub title: String,
    pub body: String,
    pub author_id: String,
    pub is_pinned: bool,
    pub is_locked: bool,
    pub answered_comment_id: Option<String>,
    pub comments_count: i32,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscussionDto {
    pub id: String,
    pub repo_id: String,
    pub category: DiscussionCategory,
    pub number: i32,
    pub title: String,
    pub body: String,
    pub author: UserDto,
    pub is_pinned: bool,
    pub is_locked: bool,
    pub answered_comment_id: Option<String>,
    pub comments_count: i32,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscussionComment {
    pub id: String,
    pub discussion_id: String,
    pub parent_comment_id: Option<String>,
    pub author_id: String,
    pub body: String,
    pub is_answer: bool,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscussionCommentDto {
    pub id: String,
    pub discussion_id: String,
    pub parent_comment_id: Option<String>,
    pub author: UserDto,
    pub body: String,
    pub is_answer: bool,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
    pub reactions: Vec<DiscussionReactionDto>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscussionReaction {
    pub id: String,
    pub subject_type: String, // "DISCUSSION" or "COMMENT"
    pub subject_id: String,
    pub user_id: String,
    pub reaction: String, // "+1", "-1", "heart", "smile", "rocket", "eyes"
    pub created_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscussionReactionDto {
    pub reaction: String,
    pub count: usize,
    pub user_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscussionSubscription {
    pub user_id: String,
    pub discussion_id: String,
    pub state: String, // "SUBSCRIBED", "IGNORED"
    pub updated_at_nanos: i64,
}

// -----------------------------------------------------------------------------
// Discovery, Topics, Languages, Profiles (Batch 13)
// -----------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtendedUserProfile {
    pub user_id: String,
    pub bio: Option<String>,
    pub company: Option<String>,
    pub location: Option<String>,
    pub website_url: Option<String>,
    pub twitter_username: Option<String>,
    pub profile_readme: Option<String>,
    pub pinned_repo_ids: Vec<String>,
    pub updated_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryTopic {
    pub id: String,
    pub repo_id: String,
    pub topic: String,
    pub created_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RepositoryLanguage {
    pub id: String,
    pub repo_id: String,
    pub language: String,
    pub bytes: i64,
    pub percentage: f64,
    pub updated_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PopularTopicDto {
    pub topic: String,
    pub repository_count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RepositoryDiscoveryCard {
    pub id: String,
    pub owner_name: String,
    pub name: String,
    pub description: Option<String>,
    pub is_private: bool,
    pub topics: Vec<String>,
    pub primary_language: Option<String>,
    pub languages: Vec<RepositoryLanguage>,
    pub stars_count: usize,
    pub forks_count: usize,
    pub updated_at_nanos: i64,
}

// -----------------------------------------------------------------------------
// Security Center Models (Batch 13)
// -----------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityScan {
    pub id: String,
    pub repo_id: String,
    pub commit_oid: String,
    pub scan_type: String, // "SECRET_SCAN", "DEPENDENCY_AUDIT"
    pub status: String,    // "IN_PROGRESS", "COMPLETED", "FAILED"
    pub findings_count: i32,
    pub started_at_nanos: i64,
    pub completed_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretScanFinding {
    pub id: String,
    pub repo_id: String,
    pub commit_oid: String,
    pub file_path: String,
    pub line_number: i32,
    pub rule_id: String,
    pub rule_name: String,
    pub severity: String, // "CRITICAL", "HIGH", "MEDIUM", "LOW"
    pub safe_fingerprint: String, // Truncated safe hash (NEVER plaintext secret!)
    pub status: String,   // "OPEN", "RESOLVED", "FALSE_POSITIVE"
    pub created_at_nanos: i64,
    pub resolved_at_nanos: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyInventory {
    pub id: String,
    pub repo_id: String,
    pub ecosystem: String, // "CARGO"
    pub package_name: String,
    pub version: String,
    pub is_direct: bool,
    pub created_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityAdvisory {
    pub id: String,
    pub advisory_id: String, // e.g., "RUSTSEC-2023-0001"
    pub ecosystem: String,
    pub package_name: String,
    pub vulnerable_version_range: String,
    pub patched_version: Option<String>,
    pub severity: String,
    pub title: String,
    pub description: String,
    pub published_at_nanos: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyVulnerability {
    pub id: String,
    pub repo_id: String,
    pub dependency_id: String,
    pub advisory_id: String,
    pub package_name: String,
    pub affected_version: String,
    pub severity: String,
    pub patched_version: Option<String>,
    pub status: String, // "OPEN", "FIXED", "DISMISSED"
    pub created_at_nanos: i64,
    pub resolved_at_nanos: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SecuritySummaryDto {
    pub total_secrets_found: usize,
    pub open_secrets_count: usize,
    pub total_dependencies: usize,
    pub vulnerable_dependencies_count: usize,
    pub critical_count: usize,
    pub high_count: usize,
    pub medium_count: usize,
    pub low_count: usize,
}

