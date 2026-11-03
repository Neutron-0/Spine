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
