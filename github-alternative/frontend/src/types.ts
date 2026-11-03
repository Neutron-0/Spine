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
