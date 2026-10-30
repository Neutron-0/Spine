export interface RepositoryMeta {
  id: string;
  owner: string;
  name: string;
  description: string;
  is_private: boolean;
  default_branch: string;
  created_at: string;
}

export interface CommitDto {
  id: string;
  author_name: string;
  author_email: string;
  timestamp: number;
  message: string;
}
