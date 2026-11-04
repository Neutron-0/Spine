use crate::artifact::ArtifactService;
use crate::core::error::ForgeError;
use crate::core::types::*;
use crate::db::Database;
use std::sync::Arc;

pub struct ReleaseService {
    db: Arc<dyn Database>,
    artifacts: Arc<ArtifactService>,
}

impl ReleaseService {
    pub fn new(db: Arc<dyn Database>, artifacts: Arc<ArtifactService>) -> Self {
        Self { db, artifacts }
    }

    pub async fn create_release(
        &self,
        repo_id: &str,
        author_id: &str,
        req: CreateReleaseRequest,
    ) -> Result<ReleaseDto, ForgeError> {
        let tag_name = req.tag_name.trim();
        if tag_name.is_empty() {
            return Err(ForgeError::BadRequest(
                "Tag name cannot be empty".to_string(),
            ));
        }

        // Check if release with same tag exists
        if let Some(_) = self.db.get_release_by_tag(repo_id, tag_name).await? {
            return Err(ForgeError::Conflict(format!(
                "Release for tag '{}' already exists in this repository",
                tag_name
            )));
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as i64;

        let is_draft = req.is_draft.unwrap_or(false);
        let published_at = if is_draft { None } else { Some(now) };

        let release = Release {
            id: uuid::Uuid::new_v4().to_string(),
            repo_id: repo_id.to_string(),
            tag_name: tag_name.to_string(),
            target_commit_id: req.target_commit_id,
            title: req.title,
            body: req.body,
            is_draft,
            is_prerelease: req.is_prerelease.unwrap_or(false),
            author_id: author_id.to_string(),
            created_at_nanos: now,
            published_at_nanos: published_at,
            updated_at_nanos: now,
        };

        let created = self.db.create_release(release).await?;
        self.to_dto(created, vec![]).await
    }

    pub async fn get_release(&self, release_id: &str) -> Result<ReleaseDto, ForgeError> {
        let rel = self
            .db
            .get_release_by_id(release_id)
            .await?
            .ok_or_else(|| ForgeError::NotFound(format!("Release {} not found", release_id)))?;

        let assets = self.db.list_assets_for_release(release_id).await?;
        self.to_dto(rel, assets).await
    }

    pub async fn get_release_by_tag(
        &self,
        repo_id: &str,
        tag_name: &str,
    ) -> Result<ReleaseDto, ForgeError> {
        let rel = self
            .db
            .get_release_by_tag(repo_id, tag_name)
            .await?
            .ok_or_else(|| {
                ForgeError::NotFound(format!("Release with tag '{}' not found", tag_name))
            })?;

        let assets = self.db.list_assets_for_release(&rel.id).await?;
        self.to_dto(rel, assets).await
    }

    pub async fn list_releases(
        &self,
        repo_id: &str,
        include_drafts: bool,
    ) -> Result<Vec<ReleaseDto>, ForgeError> {
        let list = self
            .db
            .list_releases_for_repo(repo_id, include_drafts)
            .await?;
        let mut dtos = Vec::with_capacity(list.len());
        for rel in list {
            let assets = self.db.list_assets_for_release(&rel.id).await?;
            dtos.push(self.to_dto(rel, assets).await?);
        }
        Ok(dtos)
    }

    pub async fn update_release(
        &self,
        release_id: &str,
        req: UpdateReleaseRequest,
    ) -> Result<ReleaseDto, ForgeError> {
        let mut rel = self
            .db
            .get_release_by_id(release_id)
            .await?
            .ok_or_else(|| ForgeError::NotFound(format!("Release {} not found", release_id)))?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as i64;

        if let Some(title) = req.title {
            rel.title = title;
        }
        if let Some(body) = req.body {
            rel.body = body;
        }
        if let Some(is_prerelease) = req.is_prerelease {
            rel.is_prerelease = is_prerelease;
        }
        if let Some(is_draft) = req.is_draft {
            if rel.is_draft && !is_draft && rel.published_at_nanos.is_none() {
                rel.published_at_nanos = Some(now);
            }
            rel.is_draft = is_draft;
        }
        rel.updated_at_nanos = now;

        let updated = self.db.update_release(rel).await?;
        let assets = self.db.list_assets_for_release(release_id).await?;
        self.to_dto(updated, assets).await
    }

    pub async fn delete_release(&self, release_id: &str) -> Result<(), ForgeError> {
        self.db.delete_release(release_id).await
    }

    pub async fn upload_asset(
        &self,
        release_id: &str,
        name: &str,
        content_type: &str,
        data: &[u8],
    ) -> Result<ReleaseAssetDto, ForgeError> {
        let rel = self
            .db
            .get_release_by_id(release_id)
            .await?
            .ok_or_else(|| ForgeError::NotFound(format!("Release {} not found", release_id)))?;

        let repo = self
            .db
            .get_repository_by_id(&rel.repo_id)
            .await?
            .ok_or_else(|| ForgeError::NotFound("Repository not found".to_string()))?;

        // Ingest into unified artifact store under repository owner
        let owner_type = if repo.owner_org_id.is_some() {
            "organization"
        } else {
            "user"
        };
        let owner_id = repo.owner_org_id.as_deref().unwrap_or(&repo.owner_user_id);

        let stored = self
            .artifacts
            .store_object_bytes(
                owner_type,
                owner_id,
                name,
                Some(content_type.to_string()),
                data,
            )
            .await?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as i64;

        let asset = ReleaseAsset {
            id: uuid::Uuid::new_v4().to_string(),
            release_id: release_id.to_string(),
            name: name.to_string(),
            content_type: content_type.to_string(),
            size_bytes: data.len() as i64,
            manifest_id: stored.id,
            whole_hash: stored.whole_object_hash,
            downloads_count: 0,
            created_at_nanos: now,
        };

        let created = self.db.create_release_asset(asset).await?;
        Ok(ReleaseAssetDto::from(&created))
    }

    pub async fn download_asset(
        &self,
        asset_id: &str,
    ) -> Result<(ReleaseAsset, Vec<u8>), ForgeError> {
        let mut asset =
            self.db.get_release_asset(asset_id).await?.ok_or_else(|| {
                ForgeError::NotFound(format!("Release asset {} not found", asset_id))
            })?;

        self.db.increment_asset_downloads(asset_id).await?;
        asset.downloads_count += 1;

        let bytes = self
            .artifacts
            .reconstruct_object(&asset.manifest_id)
            .await?;
        Ok((asset, bytes))
    }

    pub async fn delete_asset(&self, asset_id: &str) -> Result<(), ForgeError> {
        self.db.delete_release_asset(asset_id).await
    }

    async fn to_dto(
        &self,
        rel: Release,
        assets: Vec<ReleaseAsset>,
    ) -> Result<ReleaseDto, ForgeError> {
        let user = self
            .db
            .get_user_by_id(&rel.author_id)
            .await?
            .ok_or_else(|| ForgeError::NotFound("Author user not found".to_string()))?;

        Ok(ReleaseDto {
            id: rel.id,
            repo_id: rel.repo_id,
            tag_name: rel.tag_name,
            target_commit_id: rel.target_commit_id,
            title: rel.title,
            body: rel.body,
            is_draft: rel.is_draft,
            is_prerelease: rel.is_prerelease,
            author: UserDto::from(&user),
            assets: assets.iter().map(ReleaseAssetDto::from).collect(),
            created_at_nanos: rel.created_at_nanos,
            published_at_nanos: rel.published_at_nanos,
        })
    }
}
