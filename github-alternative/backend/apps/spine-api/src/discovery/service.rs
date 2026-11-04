use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use chrono::Utc;
use uuid::Uuid;

use crate::core::error::ForgeError;
use crate::core::types::{
    ExtendedUserProfile, PopularTopicDto, RepositoryDiscoveryCard, RepositoryLanguage,
    RepositoryTopic,
};
use crate::db::Database;
use crate::permissions::PermissionService;

pub struct DiscoveryService {
    db: Arc<dyn Database>,
    perms: Arc<PermissionService>,
    storage_root: PathBuf,
}

impl DiscoveryService {
    pub fn new(db: Arc<dyn Database>, perms: Arc<PermissionService>, storage_root: PathBuf) -> Self {
        Self {
            db,
            perms,
            storage_root,
        }
    }

    // -------------------------------------------------------------------------
    // User Profiles
    // -------------------------------------------------------------------------
    pub async fn get_user_profile(&self, username: &str) -> Result<ExtendedUserProfile, ForgeError> {
        let user = self
            .db
            .get_user_by_username(username)
            .await?
            .ok_or_else(|| ForgeError::NotFound(format!("User '{}' not found", username)))?;

        if let Some(prof) = self.db.get_extended_user_profile(&user.id).await? {
            Ok(prof)
        } else {
            Ok(ExtendedUserProfile {
                user_id: user.id,
                bio: None,
                company: None,
                location: None,
                website_url: None,
                twitter_username: None,
                profile_readme: None,
                pinned_repo_ids: Vec::new(),
                updated_at_nanos: 0,
            })
        }
    }

    pub async fn update_user_profile(
        &self,
        actor_id: &str,
        bio: Option<&str>,
        company: Option<&str>,
        location: Option<&str>,
        website_url: Option<&str>,
        twitter_username: Option<&str>,
        profile_readme: Option<&str>,
        pinned_repo_ids: Option<Vec<String>>,
    ) -> Result<ExtendedUserProfile, ForgeError> {
        let existing = self.db.get_extended_user_profile(actor_id).await?.unwrap_or_else(|| {
            ExtendedUserProfile {
                user_id: actor_id.to_string(),
                bio: None,
                company: None,
                location: None,
                website_url: None,
                twitter_username: None,
                profile_readme: None,
                pinned_repo_ids: Vec::new(),
                updated_at_nanos: 0,
            }
        });

        let updated = ExtendedUserProfile {
            user_id: actor_id.to_string(),
            bio: bio.map(|s| s.to_string()).or(existing.bio),
            company: company.map(|s| s.to_string()).or(existing.company),
            location: location.map(|s| s.to_string()).or(existing.location),
            website_url: website_url.map(|s| s.to_string()).or(existing.website_url),
            twitter_username: twitter_username.map(|s| s.to_string()).or(existing.twitter_username),
            profile_readme: profile_readme.map(|s| s.to_string()).or(existing.profile_readme),
            pinned_repo_ids: pinned_repo_ids.unwrap_or(existing.pinned_repo_ids),
            updated_at_nanos: Utc::now().timestamp_nanos_opt().unwrap_or(0),
        };

        self.db.upsert_extended_user_profile(updated).await
    }

    // -------------------------------------------------------------------------
    // Topics
    // -------------------------------------------------------------------------
    pub async fn set_repository_topics(
        &self,
        actor_id: &str,
        repo_id: &str,
        topics: &[String],
    ) -> Result<Vec<RepositoryTopic>, ForgeError> {
        self.perms.require_repo_write(actor_id, repo_id).await?;

        // Normalize and validate topics
        let mut normalized = Vec::new();
        for t in topics {
            let clean = t.trim().to_lowercase();
            if clean.is_empty() || clean.len() > 35 {
                continue;
            }
            // Must contain only lowercase alphanumeric and dashes
            if clean.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
                if !normalized.contains(&clean) {
                    normalized.push(clean);
                }
            }
        }

        self.db.set_repository_topics(repo_id, &normalized).await
    }

    pub async fn list_repository_topics(&self, repo_id: &str) -> Result<Vec<RepositoryTopic>, ForgeError> {
        self.db.list_repository_topics(repo_id).await
    }

    pub async fn list_popular_topics(&self, limit: usize) -> Result<Vec<PopularTopicDto>, ForgeError> {
        self.db.list_popular_topics(limit).await
    }

    // -------------------------------------------------------------------------
    // Language Analysis
    // -------------------------------------------------------------------------
    pub async fn analyze_repository_languages(&self, repo_id: &str) -> Result<Vec<RepositoryLanguage>, ForgeError> {
        let repo_dir = self.storage_root.join(repo_id);
        let mut byte_counts: HashMap<String, i64> = HashMap::new();

        if repo_dir.exists() {
            calculate_dir_languages(&repo_dir, &mut byte_counts);
        }

        let total_bytes: i64 = byte_counts.values().sum();
        let now = Utc::now().timestamp_nanos_opt().unwrap_or(0);

        let mut results = Vec::new();
        for (lang, bytes) in byte_counts {
            let percentage = if total_bytes > 0 {
                (bytes as f64 / total_bytes as f64) * 100.0
            } else {
                0.0
            };

            results.push(RepositoryLanguage {
                id: Uuid::new_v4().to_string(),
                repo_id: repo_id.to_string(),
                language: lang,
                bytes,
                percentage,
                updated_at_nanos: now,
            });
        }

        results.sort_by(|a, b| b.bytes.cmp(&a.bytes));
        self.db.set_repository_languages(repo_id, &results).await
    }

    pub async fn list_repository_languages(&self, repo_id: &str) -> Result<Vec<RepositoryLanguage>, ForgeError> {
        let langs = self.db.list_repository_languages(repo_id).await?;
        if langs.is_empty() {
            self.analyze_repository_languages(repo_id).await
        } else {
            Ok(langs)
        }
    }

    // -------------------------------------------------------------------------
    // Explore Feed
    // -------------------------------------------------------------------------
    pub async fn list_explore_repositories(&self, topic_filter: Option<&str>) -> Result<Vec<RepositoryDiscoveryCard>, ForgeError> {
        let repos = if let Some(t) = topic_filter {
            self.db.list_repositories_by_topic(t).await?
        } else {
            self.db.list_public_repositories().await?
        };

        let mut cards = Vec::new();
        for r in repos {
            if r.is_private {
                continue;
            }

            let topics = self
                .db
                .list_repository_topics(&r.id)
                .await?
                .into_iter()
                .map(|t| t.topic)
                .collect();

            let languages = self.db.list_repository_languages(&r.id).await.unwrap_or_default();
            let primary_language = languages.first().map(|l| l.language.clone());

            cards.push(RepositoryDiscoveryCard {
                id: r.id,
                owner_name: r.owner_username,
                name: r.name,
                description: if r.description.is_empty() { None } else { Some(r.description) },
                is_private: r.is_private,
                topics,
                primary_language,
                languages,
                stars_count: 0,
                forks_count: 0,
                updated_at_nanos: r.updated_at_nanos,
            });
        }

        cards.sort_by(|a, b| b.updated_at_nanos.cmp(&a.updated_at_nanos));
        Ok(cards)
    }
}

fn calculate_dir_languages(root: &Path, byte_counts: &mut HashMap<String, i64>) {
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let read_dir = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };

        for entry in read_dir.flatten() {
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();

            if file_name.starts_with('.') || file_name == "target" || file_name == "node_modules" {
                continue;
            }

            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                if let Ok(meta) = entry.metadata() {
                    let size = meta.len() as i64;
                    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        let lang = match ext.to_lowercase().as_str() {
                            "rs" => "Rust",
                            "ts" => "TypeScript",
                            "tsx" => "TypeScript (TSX)",
                            "js" => "JavaScript",
                            "jsx" => "JavaScript (JSX)",
                            "py" => "Python",
                            "go" => "Go",
                            "c" | "h" => "C",
                            "cpp" | "hpp" | "cc" => "C++",
                            "html" | "htm" => "HTML",
                            "css" | "scss" => "CSS",
                            "json" => "JSON",
                            "toml" => "TOML",
                            "yaml" | "yml" => "YAML",
                            "md" => "Markdown",
                            "sh" | "bash" => "Shell",
                            "sql" => "SQL",
                            _ => continue,
                        };
                        *byte_counts.entry(lang.to_string()).or_insert(0) += size;
                    }
                }
            }
        }
    }
}
