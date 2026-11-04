export interface UserDto {
  id: string;
  username: string;
  email: string;
  display_name: string;
  account_status: string;
  avatar_url?: string | null;
  created_at_nanos: number;
}

export interface SessionResponse {
  authenticated: boolean;
  user?: UserDto | null;
  session_id?: string | null;
}

export interface RepositoryDto {
  id: string;
  owner: string;
  owner_org_id?: string | null;
  name: string;
  description: string;
  is_private: boolean;
  default_branch: string;
  native_repo_id: string;
  durability_policy: string;
  provisioning_status: string;
  created_at_nanos: number;
  updated_at_nanos: number;
}

export interface BranchDto {
  name: string;
  commit_id: string;
}

export interface TagDto {
  name: string;
  target_id: string;
}

export interface CommitDto {
  id: string;
  parents: string[];
  author_name: string;
  author_email: string;
  timestamp_nanos: number;
  message: string;
}

export interface TreeEntryDto {
  name: string;
  path: string;
  is_dir: boolean;
  object_id: string;
  mode: number;
}

export interface BlobContentDto {
  path: string;
  size: number;
  is_binary: boolean;
  content: string | null;
  object_id: string;
}

export interface PatDto {
  id: string;
  name: string;
  scopes: string[];
  expires_at_nanos: number;
  created_at_nanos: number;
  last_used_at_nanos?: number | null;
  revoked: boolean;
}

// -----------------------------------------------------------------------------
// Batch 09 Collaboration Types
// -----------------------------------------------------------------------------

export interface OrganizationDto {
  id: string;
  name: string;
  display_name: string;
  description: string;
  avatar_url?: string | null;
  owner_user_id: string;
  is_private: boolean;
  created_at_nanos: number;
}

export interface OrganizationMemberDto {
  user: UserDto;
  role: 'Owner' | 'Admin' | 'Member';
  created_at_nanos: number;
}

export interface OrganizationInvitationDto {
  id: string;
  org_id: string;
  inviter_user_id: string;
  invitee_email: string;
  role: 'Owner' | 'Admin' | 'Member';
  status: string;
  expires_at_nanos: number;
  created_at_nanos: number;
}

export interface TeamDto {
  id: string;
  org_id: string;
  name: string;
  normalized_slug: string;
  description: string;
  created_at_nanos: number;
}

export interface TeamMemberDto {
  user: UserDto;
  is_maintainer: boolean;
  created_at_nanos: number;
}

export interface MilestoneDto {
  id: string;
  title: string;
  description: string;
  state: string;
  due_date_nanos?: number | null;
  created_at_nanos: number;
}

export interface LabelDto {
  id: string;
  name: string;
  color: string;
  description: string;
}

export interface IssueDto {
  id: string;
  repo_id: string;
  issue_number: number;
  author: UserDto;
  title: string;
  body: string;
  status: string;
  assignees: UserDto[];
  labels: LabelDto[];
  milestone?: MilestoneDto | null;
  comments_count: number;
  created_at_nanos: number;
  updated_at_nanos: number;
  closed_at_nanos?: number | null;
}

export interface IssueCommentDto {
  id: string;
  author: UserDto;
  body: string;
  created_at_nanos: number;
  updated_at_nanos: number;
}

export interface PullRequestDto {
  id: string;
  repo_id: string;
  pr_number: number;
  author: UserDto;
  title: string;
  body: string;
  base_branch: string;
  head_branch: string;
  base_commit_id: string;
  head_commit_id: string;
  state: string;
  is_draft: boolean;
  merge_commit_id?: string | null;
  mergeable?: boolean | null;
  requested_reviewers: UserDto[];
  reviews_count: number;
  comments_count: number;
  created_at_nanos: number;
  updated_at_nanos: number;
  closed_at_nanos?: number | null;
  merged_at_nanos?: number | null;
}

export interface PullRequestReviewDto {
  id: string;
  reviewer: UserDto;
  commit_id: string;
  state: string;
  body: string;
  created_at_nanos: number;
}

export interface PullRequestReviewCommentDto {
  id: string;
  author: UserDto;
  file_path: string;
  commit_id: string;
  diff_side: string;
  line_number: number;
  body: string;
  is_resolved: boolean;
  is_outdated: boolean;
  created_at_nanos: number;
}

export interface DiffFileDto {
  path: string;
  old_path?: string | null;
  change_type: string;
  is_binary: boolean;
  patch: string;
  additions: number;
  deletions: number;
}

export interface PullRequestDiffDto {
  base_commit_id: string;
  head_commit_id: string;
  merge_base_commit_id: string;
  files: DiffFileDto[];
  total_additions: number;
  total_deletions: number;
  commits_count: number;
}

export interface MergeabilityReport {
  mergeable: boolean;
  status: string;
  conflict_files: string[];
  base_commit_id: string;
  head_commit_id: string;
}

export interface MergeResultDto {
  success: boolean;
  merge_commit_id?: string | null;
  message: string;
}

export interface BranchProtectionRule {
  id: string;
  repo_id: string;
  branch_pattern: string;
  require_pull_request: boolean;
  required_approvals: number;
  prevent_self_approval: boolean;
  require_changes_addressed: boolean;
  dismiss_stale_reviews_on_push: boolean;
  restrict_merges_to_admins: boolean;
  created_at_nanos: number;
  updated_at_nanos: number;
}

export interface NotificationDto {
  id: string;
  reason: string;
  resource_type: string;
  resource_id: string;
  title: string;
  message: string;
  is_read: boolean;
  created_at_nanos: number;
}

export interface RepositoryActivityEvent {
  id: string;
  repo_id: string;
  actor_id?: string | null;
  event_type: string;
  payload: any;
  created_at_nanos: number;
}

// -------------------------------------------------------------
// Batch 11: Releases, Assets, Packages, Artifacts & Quota
// -------------------------------------------------------------

export interface ReleaseAssetDto {
  id: string;
  release_id: string;
  name: string;
  content_type: string;
  size_bytes: number;
  manifest_id: string;
  whole_hash: string;
  downloads_count: number;
  created_at_nanos: number;
}

export interface ReleaseDto {
  id: string;
  repo_id: string;
  tag_name: string;
  target_commit_id: string;
  title: string;
  body: string;
  is_draft: boolean;
  is_prerelease: boolean;
  author: UserDto;
  assets: ReleaseAssetDto[];
  created_at_nanos: number;
  published_at_nanos?: number | null;
}

export interface PackageVersionDto {
  id: string;
  package_id: string;
  version: string;
  manifest_id: string;
  cksum: string;
  size_bytes: number;
  metadata_json: string;
  is_yanked: boolean;
  created_by_user_id: string;
  created_at_nanos: number;
}

export interface PackageDto {
  id: string;
  ecosystem: string;
  name: string;
  owner_type: string;
  owner_id: string;
  is_private: boolean;
  description: string;
  versions: PackageVersionDto[];
  versions_count: number;
  created_at_nanos: number;
  updated_at_nanos: number;
}

export interface ArtifactQuota {
  id: string;
  owner_type: string;
  owner_id: string;
  logical_quota_bytes: number;
  used_logical_bytes: number;
  used_physical_bytes: number;
  created_at_nanos: number;
  updated_at_nanos: number;
}

export interface ArtifactNamespaceDto {
  id: string;
  name: string;
  owner_type: string;
  owner_id: string;
  is_private: boolean;
  description: string;
  created_at_nanos: number;
  updated_at_nanos: number;
}


// -------------------------------------------------------------
// Batch 12: Workflows, Actions, Webhooks, Runners & Secrets
// -------------------------------------------------------------

export interface WebhookSubscriptionDto {
  id: string;
  owner_type: string;
  owner_id: string;
  target_url: string;
  events: string[];
  is_active: boolean;
  created_at_nanos: number;
  updated_at_nanos: number;
}

export interface WebhookDeliveryDto {
  id: string;
  subscription_id: string;
  event_type: string;
  payload: any;
  response_status?: number | null;
  response_body?: string | null;
  execution_duration_ms: number;
  attempt: number;
  status: string;
  delivered_at_nanos: number;
}

export interface WorkflowDefinitionDto {
  id: string;
  repo_id: string;
  path: string;
  name: string;
  yaml_content: string;
  digest: string;
  created_at_nanos: number;
  updated_at_nanos: number;
}

export interface WorkflowRunDto {
  id: string;
  repo_id: string;
  workflow_id: string;
  workflow_name: string;
  commit_oid: string;
  ref_name: string;
  trigger_event: string;
  actor_id: string;
  status: string;
  conclusion?: string | null;
  definition_yaml: string;
  definition_digest: string;
  created_at_nanos: number;
  started_at_nanos?: number | null;
  completed_at_nanos?: number | null;
}

export interface WorkflowJobDto {
  id: string;
  run_id: string;
  repo_id: string;
  job_key: string;
  name: string;
  status: string;
  runner_id?: string | null;
  runs_on: string[];
  needs: string[];
  attempt: number;
  fencing_token: number;
  lease_expires_at_nanos?: number | null;
  exit_code?: number | null;
  created_at_nanos: number;
  started_at_nanos?: number | null;
  completed_at_nanos?: number | null;
}

export interface WorkflowStepDto {
  id: string;
  job_id: string;
  step_index: number;
  name: string;
  status: string;
  exit_code?: number | null;
  started_at_nanos?: number | null;
  completed_at_nanos?: number | null;
}

export interface WorkflowLogChunkDto {
  id: string;
  job_id: string;
  step_index: number;
  stream: string;
  content: string;
  timestamp_nanos: number;
}

export interface RunnerRegistrationDto {
  id: string;
  name: string;
  hostname: string;
  token_hash: string;
  os: string;
  arch: string;
  labels: string[];
  max_concurrency: number;
  active_jobs: number;
  status: string;
  last_heartbeat_at_nanos: number;
  created_at_nanos: number;
}

export interface SecretSummaryDto {
  id: string;
  name: string;
  owner_type: string;
  owner_id: string;
  created_at_nanos: number;
  updated_at_nanos: number;
}

// -----------------------------------------------------------------------------
// Search Models (Batch 13)
// -----------------------------------------------------------------------------

export type SearchResultType = 'Repository' | 'Code' | 'Issue' | 'PullRequest' | 'Discussion';

export interface SearchHitDto {
  result_type: SearchResultType;
  repo_id: string;
  repo_name: string;
  title: string;
  snippet: string;
  file_path?: string | null;
  line_number?: number | null;
  url: string;
  score: number;
}

export interface SearchResponseDto {
  query: string;
  total: number;
  hits: SearchHitDto[];
}

// -----------------------------------------------------------------------------
// Discussion Models (Batch 13)
// -----------------------------------------------------------------------------

export interface DiscussionCategoryDto {
  id: string;
  repo_id: string;
  name: string;
  slug: string;
  description?: string | null;
  emoji: string;
  is_answerable: boolean;
  created_at_nanos: number;
}

export interface DiscussionReactionDto {
  reaction: string;
  count: number;
  user_ids: string[];
}

export interface DiscussionCommentDto {
  id: string;
  discussion_id: string;
  parent_comment_id?: string | null;
  author: UserDto;
  body: string;
  is_answer: boolean;
  created_at_nanos: number;
  updated_at_nanos: number;
  reactions: DiscussionReactionDto[];
}

export interface DiscussionDto {
  id: string;
  repo_id: string;
  category: DiscussionCategoryDto;
  number: number;
  title: string;
  body: string;
  author: UserDto;
  is_pinned: boolean;
  is_locked: boolean;
  answered_comment_id?: string | null;
  comments_count: number;
  created_at_nanos: number;
  updated_at_nanos: number;
}

// -----------------------------------------------------------------------------
// Discovery, Topics, Languages, Profiles (Batch 13)
// -----------------------------------------------------------------------------

export interface ExtendedUserProfileDto {
  user_id: string;
  bio?: string | null;
  company?: string | null;
  location?: string | null;
  website_url?: string | null;
  twitter_username?: string | null;
  profile_readme?: string | null;
  pinned_repo_ids: string[];
  updated_at_nanos: number;
}

export interface RepositoryTopicDto {
  id: string;
  repo_id: string;
  topic: string;
  created_at_nanos: number;
}

export interface RepositoryLanguageDto {
  id: string;
  repo_id: string;
  language: string;
  bytes: number;
  percentage: number;
  updated_at_nanos: number;
}

export interface PopularTopicDto {
  topic: string;
  repository_count: number;
}

export interface RepositoryDiscoveryCardDto {
  id: string;
  owner_name: string;
  name: string;
  description?: string | null;
  is_private: boolean;
  topics: string[];
  primary_language?: string | null;
  languages: RepositoryLanguageDto[];
  stars_count: number;
  forks_count: number;
  updated_at_nanos: number;
}

// -----------------------------------------------------------------------------
// Security Center (Batch 13)
// -----------------------------------------------------------------------------

export interface SecurityScanDto {
  id: string;
  repo_id: string;
  commit_oid: string;
  scan_type: string;
  status: string;
  findings_count: number;
  started_at_nanos: number;
  completed_at_nanos: number;
}

export interface SecretScanFindingDto {
  id: string;
  repo_id: string;
  commit_oid: string;
  file_path: string;
  line_number: number;
  rule_id: string;
  rule_name: string;
  severity: string;
  safe_fingerprint: string;
  status: string;
  created_at_nanos: number;
  resolved_at_nanos?: number | null;
}

export interface DependencyInventoryDto {
  id: string;
  repo_id: string;
  ecosystem: string;
  package_name: string;
  version: string;
  is_direct: boolean;
  created_at_nanos: number;
}

export interface DependencyVulnerabilityDto {
  id: string;
  repo_id: string;
  dependency_id: string;
  advisory_id: string;
  package_name: string;
  affected_version: string;
  severity: string;
  patched_version?: string | null;
  status: string;
  created_at_nanos: number;
  resolved_at_nanos?: number | null;
}

export interface SecuritySummaryDto {
  total_secrets_found: number;
  open_secrets_count: number;
  total_dependencies: number;
  vulnerable_dependencies_count: number;
  critical_count: number;
  high_count: number;
  medium_count: number;
  low_count: number;
}
