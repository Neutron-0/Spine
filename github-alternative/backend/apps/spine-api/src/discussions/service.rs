use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

use crate::core::error::ForgeError;
use crate::core::types::{
    Discussion, DiscussionCategory, DiscussionComment, DiscussionCommentDto, DiscussionDto,
    DiscussionReaction, DiscussionReactionDto, DiscussionSubscription, UserDto,
};
use crate::db::Database;
use crate::permissions::PermissionService;

pub struct DiscussionService {
    db: Arc<dyn Database>,
    perms: Arc<PermissionService>,
}

impl DiscussionService {
    pub fn new(db: Arc<dyn Database>, perms: Arc<PermissionService>) -> Self {
        Self { db, perms }
    }

    /// Ensure default discussion categories exist for a repository.
    pub async fn ensure_default_categories(
        &self,
        repo_id: &str,
    ) -> Result<Vec<DiscussionCategory>, ForgeError> {
        let existing = self.db.list_discussion_categories(repo_id).await?;
        if !existing.is_empty() {
            return Ok(existing);
        }

        let now = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let defaults = vec![
            (
                "Announcements",
                "announcements",
                "Official updates and news",
                "📢",
                false,
            ),
            (
                "General",
                "general",
                "Anything and everything related to the project",
                "💬",
                false,
            ),
            (
                "Q&A",
                "q-and-a",
                "Ask questions and get answers from the community",
                "❓",
                true,
            ),
            (
                "Ideas",
                "ideas",
                "Feature requests and architecture proposals",
                "💡",
                false,
            ),
            (
                "Show and tell",
                "show-and-tell",
                "Show off what you've built with Spine",
                "🙌",
                false,
            ),
        ];

        let mut created = Vec::new();
        for (name, slug, desc, emoji, is_ans) in defaults {
            let cat = DiscussionCategory {
                id: Uuid::new_v4().to_string(),
                repo_id: repo_id.to_string(),
                name: name.to_string(),
                slug: slug.to_string(),
                description: Some(desc.to_string()),
                emoji: emoji.to_string(),
                is_answerable: is_ans,
                created_at_nanos: now,
            };
            let c = self.db.create_discussion_category(cat).await?;
            created.push(c);
        }

        Ok(created)
    }

    pub async fn list_categories(
        &self,
        repo_id: &str,
    ) -> Result<Vec<DiscussionCategory>, ForgeError> {
        let categories = self.db.list_discussion_categories(repo_id).await?;
        if categories.is_empty() {
            self.ensure_default_categories(repo_id).await
        } else {
            Ok(categories)
        }
    }

    pub async fn create_category(
        &self,
        actor_id: &str,
        repo_id: &str,
        name: &str,
        slug: &str,
        description: Option<&str>,
        emoji: &str,
        is_answerable: bool,
    ) -> Result<DiscussionCategory, ForgeError> {
        self.perms.require_repo_write(actor_id, repo_id).await?;
        let cat = DiscussionCategory {
            id: Uuid::new_v4().to_string(),
            repo_id: repo_id.to_string(),
            name: name.to_string(),
            slug: slug.to_string(),
            description: description.map(|s| s.to_string()),
            emoji: if emoji.is_empty() {
                "💬".to_string()
            } else {
                emoji.to_string()
            },
            is_answerable,
            created_at_nanos: Utc::now().timestamp_nanos_opt().unwrap_or(0),
        };
        self.db.create_discussion_category(cat).await
    }

    pub async fn create_discussion(
        &self,
        author_id: &str,
        repo_id: &str,
        category_id: &str,
        title: &str,
        body: &str,
    ) -> Result<DiscussionDto, ForgeError> {
        self.perms
            .require_repo_read(Some(author_id), repo_id)
            .await?;

        let cat = self
            .db
            .get_discussion_category(category_id)
            .await?
            .ok_or_else(|| {
                ForgeError::NotFound(format!("Discussion category '{}' not found", category_id))
            })?;

        let now = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let disc = Discussion {
            id: Uuid::new_v4().to_string(),
            repo_id: repo_id.to_string(),
            category_id: category_id.to_string(),
            number: 0, // Handled atomically by DB
            title: title.to_string(),
            body: body.to_string(),
            author_id: author_id.to_string(),
            is_pinned: false,
            is_locked: false,
            answered_comment_id: None,
            comments_count: 0,
            created_at_nanos: now,
            updated_at_nanos: now,
        };

        let created = self.db.create_discussion(disc).await?;

        // Automatically subscribe author
        let _ = self
            .db
            .set_discussion_subscription(DiscussionSubscription {
                user_id: author_id.to_string(),
                discussion_id: created.id.clone(),
                state: "SUBSCRIBED".to_string(),
                updated_at_nanos: now,
            })
            .await;

        self.enrich_discussion(created, cat).await
    }

    pub async fn get_discussion_by_number(
        &self,
        user_id: Option<&str>,
        repo_id: &str,
        number: i32,
    ) -> Result<DiscussionDto, ForgeError> {
        self.perms.require_repo_read(user_id, repo_id).await?;

        let disc = self
            .db
            .get_discussion_by_number(repo_id, number)
            .await?
            .ok_or_else(|| ForgeError::NotFound(format!("Discussion #{} not found", number)))?;

        let cat = self
            .db
            .get_discussion_category(&disc.category_id)
            .await?
            .unwrap_or_else(|| DiscussionCategory {
                id: disc.category_id.clone(),
                repo_id: repo_id.to_string(),
                name: "General".to_string(),
                slug: "general".to_string(),
                description: None,
                emoji: "💬".to_string(),
                is_answerable: false,
                created_at_nanos: 0,
            });

        self.enrich_discussion(disc, cat).await
    }

    pub async fn list_discussions(
        &self,
        user_id: Option<&str>,
        repo_id: &str,
        category_id: Option<&str>,
    ) -> Result<Vec<DiscussionDto>, ForgeError> {
        self.perms.require_repo_read(user_id, repo_id).await?;

        let categories = self.list_categories(repo_id).await?;
        let cat_map: std::collections::HashMap<String, DiscussionCategory> =
            categories.into_iter().map(|c| (c.id.clone(), c)).collect();

        let discussions = self.db.list_discussions(repo_id, category_id).await?;
        let mut dtos = Vec::new();

        for d in discussions {
            let cat = cat_map
                .get(&d.category_id)
                .cloned()
                .unwrap_or_else(|| DiscussionCategory {
                    id: d.category_id.clone(),
                    repo_id: repo_id.to_string(),
                    name: "General".to_string(),
                    slug: "general".to_string(),
                    description: None,
                    emoji: "💬".to_string(),
                    is_answerable: false,
                    created_at_nanos: 0,
                });

            if let Ok(dto) = self.enrich_discussion(d, cat).await {
                dtos.push(dto);
            }
        }

        Ok(dtos)
    }

    pub async fn update_discussion(
        &self,
        actor_id: &str,
        repo_id: &str,
        number: i32,
        title: Option<&str>,
        body: Option<&str>,
        category_id: Option<&str>,
        is_pinned: Option<bool>,
        is_locked: Option<bool>,
    ) -> Result<DiscussionDto, ForgeError> {
        let mut disc = self
            .db
            .get_discussion_by_number(repo_id, number)
            .await?
            .ok_or_else(|| ForgeError::NotFound(format!("Discussion #{} not found", number)))?;

        let is_author = disc.author_id == actor_id;
        let is_admin = self
            .perms
            .can_admin_repo(actor_id, repo_id)
            .await
            .unwrap_or(false);

        if !is_author && !is_admin {
            return Err(ForgeError::Forbidden(
                "Not authorized to modify this discussion".to_string(),
            ));
        }

        if let Some(t) = title {
            disc.title = t.to_string();
        }
        if let Some(b) = body {
            disc.body = b.to_string();
        }
        if let Some(cid) = category_id {
            disc.category_id = cid.to_string();
        }
        if let Some(pinned) = is_pinned {
            if !is_admin {
                return Err(ForgeError::Forbidden(
                    "Only repository administrators can pin discussions".to_string(),
                ));
            }
            disc.is_pinned = pinned;
        }
        if let Some(locked) = is_locked {
            if !is_admin {
                return Err(ForgeError::Forbidden(
                    "Only repository administrators can lock discussions".to_string(),
                ));
            }
            disc.is_locked = locked;
        }

        disc.updated_at_nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let updated = self.db.update_discussion(disc).await?;

        let cat = self
            .db
            .get_discussion_category(&updated.category_id)
            .await?
            .unwrap_or_else(|| DiscussionCategory {
                id: updated.category_id.clone(),
                repo_id: repo_id.to_string(),
                name: "General".to_string(),
                slug: "general".to_string(),
                description: None,
                emoji: "💬".to_string(),
                is_answerable: false,
                created_at_nanos: 0,
            });

        self.enrich_discussion(updated, cat).await
    }

    pub async fn create_comment(
        &self,
        author_id: &str,
        repo_id: &str,
        number: i32,
        parent_comment_id: Option<&str>,
        body: &str,
    ) -> Result<DiscussionCommentDto, ForgeError> {
        self.perms
            .require_repo_read(Some(author_id), repo_id)
            .await?;

        let disc = self
            .db
            .get_discussion_by_number(repo_id, number)
            .await?
            .ok_or_else(|| ForgeError::NotFound(format!("Discussion #{} not found", number)))?;

        if disc.is_locked {
            return Err(ForgeError::Conflict(
                "Discussion is locked for new comments".to_string(),
            ));
        }

        let now = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let comment = DiscussionComment {
            id: Uuid::new_v4().to_string(),
            discussion_id: disc.id,
            parent_comment_id: parent_comment_id.map(|s| s.to_string()),
            author_id: author_id.to_string(),
            body: body.to_string(),
            is_answer: false,
            created_at_nanos: now,
            updated_at_nanos: now,
        };

        let created = self.db.create_discussion_comment(comment).await?;
        self.enrich_comment(created).await
    }

    pub async fn list_comments(
        &self,
        user_id: Option<&str>,
        repo_id: &str,
        number: i32,
    ) -> Result<Vec<DiscussionCommentDto>, ForgeError> {
        self.perms.require_repo_read(user_id, repo_id).await?;

        let disc = self
            .db
            .get_discussion_by_number(repo_id, number)
            .await?
            .ok_or_else(|| ForgeError::NotFound(format!("Discussion #{} not found", number)))?;

        let comments = self.db.list_discussion_comments(&disc.id).await?;
        let mut dtos = Vec::new();
        for c in comments {
            if let Ok(dto) = self.enrich_comment(c).await {
                dtos.push(dto);
            }
        }
        Ok(dtos)
    }

    pub async fn mark_answer(
        &self,
        actor_id: &str,
        repo_id: &str,
        number: i32,
        comment_id: &str,
    ) -> Result<DiscussionDto, ForgeError> {
        let mut disc = self
            .db
            .get_discussion_by_number(repo_id, number)
            .await?
            .ok_or_else(|| ForgeError::NotFound(format!("Discussion #{} not found", number)))?;

        let is_author = disc.author_id == actor_id;
        let is_admin = self
            .perms
            .can_admin_repo(actor_id, repo_id)
            .await
            .unwrap_or(false);
        if !is_author && !is_admin {
            return Err(ForgeError::Forbidden(
                "Only discussion author or repo admin can mark an answer".to_string(),
            ));
        }

        let cat = self
            .db
            .get_discussion_category(&disc.category_id)
            .await?
            .ok_or_else(|| ForgeError::NotFound("Category not found".to_string()))?;

        if !cat.is_answerable {
            return Err(ForgeError::Conflict(
                "Category does not support accepted answers".to_string(),
            ));
        }

        let mut comment = self
            .db
            .get_discussion_comment(comment_id)
            .await?
            .ok_or_else(|| ForgeError::NotFound(format!("Comment '{}' not found", comment_id)))?;

        // Clear previous answer if any
        if let Some(ref prev_id) = disc.answered_comment_id {
            if let Some(mut prev_comment) = self.db.get_discussion_comment(prev_id).await? {
                prev_comment.is_answer = false;
                let _ = self.db.update_discussion_comment(prev_comment).await;
            }
        }

        comment.is_answer = true;
        let _ = self.db.update_discussion_comment(comment).await?;

        disc.answered_comment_id = Some(comment_id.to_string());
        disc.updated_at_nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let updated = self.db.update_discussion(disc).await?;

        self.enrich_discussion(updated, cat).await
    }

    pub async fn toggle_reaction(
        &self,
        user_id: &str,
        subject_type: &str,
        subject_id: &str,
        reaction: &str,
    ) -> Result<bool, ForgeError> {
        let existing = self
            .db
            .list_discussion_reactions(subject_type, subject_id)
            .await?;
        if existing
            .iter()
            .any(|r| r.user_id == user_id && r.reaction == reaction)
        {
            self.db
                .remove_discussion_reaction(subject_type, subject_id, user_id, reaction)
                .await?;
            Ok(false) // Removed
        } else {
            let r = DiscussionReaction {
                id: Uuid::new_v4().to_string(),
                subject_type: subject_type.to_string(),
                subject_id: subject_id.to_string(),
                user_id: user_id.to_string(),
                reaction: reaction.to_string(),
                created_at_nanos: Utc::now().timestamp_nanos_opt().unwrap_or(0),
            };
            self.db.add_discussion_reaction(r).await?;
            Ok(true) // Added
        }
    }

    async fn enrich_discussion(
        &self,
        d: Discussion,
        cat: DiscussionCategory,
    ) -> Result<DiscussionDto, ForgeError> {
        let author = if let Some(u) = self.db.get_user_by_id(&d.author_id).await? {
            UserDto::from(&u)
        } else {
            UserDto {
                id: d.author_id.clone(),
                username: "unknown".to_string(),
                email: String::new(),
                display_name: "Unknown User".to_string(),
                account_status: "ACTIVE".to_string(),
                avatar_url: None,
                created_at_nanos: 0,
            }
        };

        Ok(DiscussionDto {
            id: d.id,
            repo_id: d.repo_id,
            category: cat,
            number: d.number,
            title: d.title,
            body: d.body,
            author,
            is_pinned: d.is_pinned,
            is_locked: d.is_locked,
            answered_comment_id: d.answered_comment_id,
            comments_count: d.comments_count,
            created_at_nanos: d.created_at_nanos,
            updated_at_nanos: d.updated_at_nanos,
        })
    }

    async fn enrich_comment(
        &self,
        c: DiscussionComment,
    ) -> Result<DiscussionCommentDto, ForgeError> {
        let author = if let Some(u) = self.db.get_user_by_id(&c.author_id).await? {
            UserDto::from(&u)
        } else {
            UserDto {
                id: c.author_id.clone(),
                username: "unknown".to_string(),
                email: String::new(),
                display_name: "Unknown User".to_string(),
                account_status: "ACTIVE".to_string(),
                avatar_url: None,
                created_at_nanos: 0,
            }
        };

        let raw_reactions = self
            .db
            .list_discussion_reactions("COMMENT", &c.id)
            .await
            .unwrap_or_default();
        let mut grouped: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for r in raw_reactions {
            grouped.entry(r.reaction).or_default().push(r.user_id);
        }

        let reactions = grouped
            .into_iter()
            .map(|(reaction, user_ids)| DiscussionReactionDto {
                count: user_ids.len(),
                reaction,
                user_ids,
            })
            .collect();

        Ok(DiscussionCommentDto {
            id: c.id,
            discussion_id: c.discussion_id,
            parent_comment_id: c.parent_comment_id,
            author,
            body: c.body,
            is_answer: c.is_answer,
            created_at_nanos: c.created_at_nanos,
            updated_at_nanos: c.updated_at_nanos,
            reactions,
        })
    }
}
