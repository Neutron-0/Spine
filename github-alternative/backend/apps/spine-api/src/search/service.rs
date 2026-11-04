use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::core::error::ForgeError;
use crate::core::types::{SearchHit, SearchResponse, SearchResultType};
use crate::db::Database;
use crate::permissions::PermissionService;

pub struct SearchService {
    db: Arc<dyn Database>,
    perms: Arc<PermissionService>,
    storage_root: PathBuf,
}

impl SearchService {
    pub fn new(db: Arc<dyn Database>, perms: Arc<PermissionService>, storage_root: PathBuf) -> Self {
        Self {
            db,
            perms,
            storage_root,
        }
    }

    /// Search across repositories, code, issues, pull requests, and discussions with strict permission gating.
    pub async fn search(
        &self,
        query: &str,
        result_type: Option<SearchResultType>,
        repo_filter: Option<&str>,
        user_id: Option<&str>,
    ) -> Result<SearchResponse, ForgeError> {
        let trimmed_query = query.trim();
        if trimmed_query.is_empty() {
            return Ok(SearchResponse {
                query: query.to_string(),
                total: 0,
                hits: Vec::new(),
            });
        }

        let query_lower = trimmed_query.to_lowercase();
        let mut hits = Vec::new();

        // 1. Resolve target repositories
        let repos = if let Some(rf) = repo_filter {
            if let Some((owner, name)) = rf.split_once('/') {
                if let Some(r) = self.db.get_repository_by_owner_name(owner, name).await? {
                    vec![r]
                } else {
                    Vec::new()
                }
            } else if let Some(r) = self.db.get_repository_by_id(rf).await? {
                vec![r]
            } else {
                Vec::new()
            }
        } else {
            let mut all = self.db.list_public_repositories().await.unwrap_or_default();
            if let Some(uid) = user_id {
                let user_repos = self.db.list_repositories_for_user(uid).await.unwrap_or_default();
                for r in user_repos {
                    if !all.iter().any(|existing| existing.id == r.id) {
                        all.push(r);
                    }
                }
            }
            all
        };

        // Filter repos user has read permission to
        let mut authorized_repos = Vec::new();
        for r in repos {
            if self.perms.can_read_repo(user_id, &r.id).await.unwrap_or(false) {
                authorized_repos.push(r);
            }
        }

        let search_all = result_type.is_none();

        for repo in &authorized_repos {
            let repo_full_name = format!("{}/{}", repo.owner_username, repo.name);

            // A. Repository metadata search
            if search_all || result_type == Some(SearchResultType::Repository) {
                let mut score = 0.0;
                if repo.name.to_lowercase().contains(&query_lower) {
                    score += 10.0;
                }
                if repo.description.to_lowercase().contains(&query_lower) {
                    score += 3.0;
                }

                // Check topics
                let topics = self.db.list_repository_topics(&repo.id).await.unwrap_or_default();
                for t in &topics {
                    if t.topic.contains(&query_lower) {
                        score += 5.0;
                    }
                }

                if score > 0.0 {
                    hits.push(SearchHit {
                        result_type: SearchResultType::Repository,
                        repo_id: repo.id.clone(),
                        repo_name: repo_full_name.clone(),
                        title: repo_full_name.clone(),
                        snippet: if repo.description.is_empty() { "Repository".to_string() } else { repo.description.clone() },
                        file_path: None,
                        line_number: None,
                        url: format!("/{}", repo_full_name),
                        score,
                    });
                }
            }

            // B. Issues search
            if search_all || result_type == Some(SearchResultType::Issue) {
                let issues = self.db.list_issues_for_repo(&repo.id, None).await.unwrap_or_default();
                for issue in issues {
                    let mut score = 0.0;
                    if issue.title.to_lowercase().contains(&query_lower) {
                        score += 8.0;
                    }
                    if issue.body.to_lowercase().contains(&query_lower) {
                        score += 2.0;
                    }

                    if score > 0.0 {
                        hits.push(SearchHit {
                            result_type: SearchResultType::Issue,
                            repo_id: repo.id.clone(),
                            repo_name: repo_full_name.clone(),
                            title: format!("#{} {}", issue.issue_number, issue.title),
                            snippet: extract_snippet(&issue.body, &query_lower),
                            file_path: None,
                            line_number: None,
                            url: format!("/{}/issues/{}", repo_full_name, issue.issue_number),
                            score,
                        });
                    }
                }
            }

            // C. Pull requests search
            if search_all || result_type == Some(SearchResultType::PullRequest) {
                let prs = self.db.list_pull_requests_for_repo(&repo.id, None).await.unwrap_or_default();
                for pr in prs {
                    let mut score = 0.0;
                    if pr.title.to_lowercase().contains(&query_lower) {
                        score += 8.0;
                    }
                    if pr.body.to_lowercase().contains(&query_lower) {
                        score += 2.0;
                    }

                    if score > 0.0 {
                        hits.push(SearchHit {
                            result_type: SearchResultType::PullRequest,
                            repo_id: repo.id.clone(),
                            repo_name: repo_full_name.clone(),
                            title: format!("#{} {}", pr.pr_number, pr.title),
                            snippet: extract_snippet(&pr.body, &query_lower),
                            file_path: None,
                            line_number: None,
                            url: format!("/{}/pulls/{}", repo_full_name, pr.pr_number),
                            score,
                        });
                    }
                }
            }

            // D. Discussions search
            if search_all || result_type == Some(SearchResultType::Discussion) {
                let discussions = self.db.list_discussions(&repo.id, None).await.unwrap_or_default();
                for disc in discussions {
                    let mut score = 0.0;
                    if disc.title.to_lowercase().contains(&query_lower) {
                        score += 8.0;
                    }
                    if disc.body.to_lowercase().contains(&query_lower) {
                        score += 2.0;
                    }

                    if score > 0.0 {
                        hits.push(SearchHit {
                            result_type: SearchResultType::Discussion,
                            repo_id: repo.id.clone(),
                            repo_name: repo_full_name.clone(),
                            title: format!("#{} {}", disc.number, disc.title),
                            snippet: extract_snippet(&disc.body, &query_lower),
                            file_path: None,
                            line_number: None,
                            url: format!("/{}/discussions/{}", repo_full_name, disc.number),
                            score,
                        });
                    }
                }
            }

            // E. Code search in repository tree
            if search_all || result_type == Some(SearchResultType::Code) {
                let repo_dir = self.storage_root.join(&repo.id);
                if repo_dir.exists() {
                    let code_hits = search_code_in_dir(&repo_dir, &repo.id, &repo_full_name, &query_lower);
                    hits.extend(code_hits);
                }
            }
        }

        // Sort descending by score
        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        hits.truncate(100);

        let total = hits.len();
        Ok(SearchResponse {
            query: query.to_string(),
            total,
            hits,
        })
    }
}

fn extract_snippet(text: &str, query: &str) -> String {
    if let Some(idx) = text.to_lowercase().find(query) {
        let start = idx.saturating_sub(40);
        let end = std::cmp::min(idx + query.len() + 60, text.len());
        let prefix = if start > 0 { "..." } else { "" };
        let suffix = if end < text.len() { "..." } else { "" };
        format!("{}{}{}", prefix, &text[start..end], suffix)
    } else if text.len() > 100 {
        format!("{}...", &text[..100])
    } else {
        text.to_string()
    }
}

fn search_code_in_dir(root: &Path, repo_id: &str, repo_name: &str, query_lower: &str) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let read_dir = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };

        for entry in read_dir.flatten() {
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();

            // Skip hidden or metadata directories
            if file_name.starts_with('.') || file_name == "target" || file_name == "node_modules" {
                continue;
            }

            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                // Check if file is text/code
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let rel_path = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
                    for (line_idx, line) in content.lines().enumerate() {
                        if line.to_lowercase().contains(query_lower) {
                            let snippet = line.trim().to_string();
                            hits.push(SearchHit {
                                result_type: SearchResultType::Code,
                                repo_id: repo_id.to_string(),
                                repo_name: repo_name.to_string(),
                                title: format!("{}:{}", rel_path, line_idx + 1),
                                snippet,
                                file_path: Some(rel_path.clone()),
                                line_number: Some(line_idx + 1),
                                url: format!("/{}/blob/main/{}#L{}", repo_name, rel_path, line_idx + 1),
                                score: 6.0,
                            });
                            if hits.len() >= 50 {
                                return hits;
                            }
                        }
                    }
                }
            }
        }
    }

    hits
}
