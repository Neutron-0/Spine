use crate::core::error::ForgeError;
use crate::core::types::*;
use crate::db::Database;
use spine_artifact::{
    chunker::ChunkerError, ArtifactManifest, ArtifactStore, ObjectReconstructor, StreamChunker,
    TransferSession, DEFAULT_CHUNK_SIZE,
};
use spine_cluster::durability::DurabilityPolicy;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct ArtifactService {
    db: Arc<dyn Database>,
    store: Arc<dyn ArtifactStore>,
    active_sessions: RwLock<HashMap<String, TransferSession>>,
}

impl ArtifactService {
    pub fn new(db: Arc<dyn Database>, store: Arc<dyn ArtifactStore>) -> Self {
        Self {
            db,
            store,
            active_sessions: RwLock::new(HashMap::new()),
        }
    }

    pub fn store(&self) -> &Arc<dyn ArtifactStore> {
        &self.store
    }

    /// Checks quota and creates a new resumable upload session.
    pub async fn create_upload_session(
        &self,
        owner_type: &str,
        owner_id: &str,
        artifact_id: &str,
        total_size: u64,
        chunk_size: u32,
        durability_policy: DurabilityPolicy,
        ttl_seconds: i64,
    ) -> Result<ArtifactUploadSession, ForgeError> {
        // 1. Quota check
        let quota = self.db.get_or_create_quota(owner_type, owner_id).await?;
        if (quota.used_logical_bytes as u64) + total_size > quota.logical_quota_bytes as u64 {
            return Err(ForgeError::BadRequest(format!(
                "Storage quota exceeded: requested {} bytes, currently using {}/{} bytes",
                total_size, quota.used_logical_bytes, quota.logical_quota_bytes
            )));
        }

        let session_id = uuid::Uuid::new_v4().to_string();
        let chunk_size = if chunk_size == 0 {
            DEFAULT_CHUNK_SIZE
        } else {
            chunk_size
        };

        let session = TransferSession::new(
            session_id.clone(),
            artifact_id.to_string(),
            total_size,
            chunk_size,
            durability_policy,
            ttl_seconds,
        );

        let db_session = ArtifactUploadSession {
            id: session_id.clone(),
            artifact_id: artifact_id.to_string(),
            owner_type: owner_type.to_string(),
            owner_id: owner_id.to_string(),
            state: format!("{:?}", session.state),
            total_size: total_size as i64,
            chunk_size: chunk_size as i32,
            expected_chunks: session.expected_chunks as i32,
            durability_policy: format!("{:?}", durability_policy),
            created_at_nanos: session.created_at_nanos,
            expires_at_nanos: session.expires_at_nanos,
        };

        self.db.create_upload_session(db_session.clone()).await?;
        self.active_sessions
            .write()
            .await
            .insert(session_id, session);

        Ok(db_session)
    }

    /// Ingests a single chunk into storage for an active session.
    pub async fn upload_chunk(
        &self,
        session_id: &str,
        chunk_index: u32,
        chunk_hash: &str,
        data: &[u8],
    ) -> Result<(), ForgeError> {
        // Validate chunk hash
        let actual_hash = blake3::hash(data).to_hex().to_string();
        if actual_hash != chunk_hash {
            return Err(ForgeError::BadRequest(format!(
                "Chunk hash mismatch for index {}: expected {}, computed {}",
                chunk_index, chunk_hash, actual_hash
            )));
        }

        // Check session exists
        let mut sessions = self.active_sessions.write().await;
        if let Some(session) = sessions.get_mut(session_id) {
            session
                .record_chunk_uploaded(chunk_index)
                .map_err(|e| ForgeError::BadRequest(e.to_string()))?;
        }

        // Store chunk payload
        self.store
            .store_chunk(chunk_hash, data)
            .map_err(|e| ForgeError::Internal(format!("Failed to store chunk: {}", e)))?;

        Ok(())
    }

    /// Finalizes an upload session by recording the verified artifact manifest and adjusting quotas.
    pub async fn finalize_upload(
        &self,
        session_id: &str,
        manifest: ArtifactManifest,
    ) -> Result<StoredArtifactManifest, ForgeError> {
        manifest
            .validate()
            .map_err(|e| ForgeError::BadRequest(format!("Invalid artifact manifest: {}", e)))?;

        let db_session = self
            .db
            .get_upload_session(session_id)
            .await?
            .ok_or_else(|| {
                ForgeError::NotFound(format!("Upload session {} not found", session_id))
            })?;

        // Put manifest in content-addressed store
        let manifest_id = self
            .store
            .store_manifest(&manifest)
            .map_err(|e| ForgeError::Internal(format!("Failed to store manifest: {}", e)))?;

        let manifest_json =
            serde_json::to_string(&manifest).map_err(|e| ForgeError::Internal(e.to_string()))?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as i64;

        let stored = StoredArtifactManifest {
            id: manifest_id.clone(),
            artifact_id: manifest.artifact_id.clone(),
            whole_object_hash: manifest.whole_object_hash.clone(),
            total_size: manifest.total_size as i64,
            chunk_size: manifest.chunk_size as i32,
            chunk_count: manifest.chunks.len() as i32,
            content_type: manifest.content_type.clone(),
            manifest_json,
            created_at_nanos: now,
        };

        // Update database
        self.db.save_artifact_manifest(stored.clone()).await?;
        self.db
            .update_upload_session_state(session_id, "Complete")
            .await?;

        // Update quota usage
        self.db
            .update_quota_usage(
                &db_session.owner_id,
                manifest.total_size as i64,
                manifest.total_size as i64,
            )
            .await?;

        // Remove from active memory map
        self.active_sessions.write().await.remove(session_id);

        Ok(stored)
    }

    /// High-level atomic object ingestion: chunks, stores, creates manifest, and updates quota.
    pub async fn store_object_bytes(
        &self,
        owner_type: &str,
        owner_id: &str,
        artifact_id: &str,
        content_type: Option<String>,
        data: &[u8],
    ) -> Result<StoredArtifactManifest, ForgeError> {
        let total_size = data.len() as u64;

        // Quota check
        let quota = self.db.get_or_create_quota(owner_type, owner_id).await?;
        if (quota.used_logical_bytes as u64) + total_size > quota.logical_quota_bytes as u64 {
            return Err(ForgeError::BadRequest(format!(
                "Storage quota exceeded: requested {} bytes, currently using {}/{} bytes",
                total_size, quota.used_logical_bytes, quota.logical_quota_bytes
            )));
        }

        // Chunk data
        let chunker = StreamChunker::new(Some(DEFAULT_CHUNK_SIZE));
        let res = chunker
            .chunk_reader(Cursor::new(data), artifact_id, content_type.clone())
            .map_err(|e| ForgeError::Internal(format!("Chunking failed: {}", e)))?;

        // Store chunks
        for chunk in &res.chunks {
            self.store
                .store_chunk(&chunk.entry.chunk_hash, &chunk.bytes)
                .map_err(|e| ForgeError::Internal(format!("Failed to store chunk: {}", e)))?;
        }

        let manifest_id = self
            .store
            .store_manifest(&res.manifest)
            .map_err(|e| ForgeError::Internal(format!("Failed to store manifest: {}", e)))?;

        let manifest_json = serde_json::to_string(&res.manifest)
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as i64;

        let stored = StoredArtifactManifest {
            id: manifest_id,
            artifact_id: artifact_id.to_string(),
            whole_object_hash: res.manifest.whole_object_hash.clone(),
            total_size: total_size as i64,
            chunk_size: DEFAULT_CHUNK_SIZE as i32,
            chunk_count: res.manifest.chunks.len() as i32,
            content_type,
            manifest_json,
            created_at_nanos: now,
        };

        self.db.save_artifact_manifest(stored.clone()).await?;

        // Update quota
        self.db
            .update_quota_usage(owner_id, total_size as i64, total_size as i64)
            .await?;

        Ok(stored)
    }

    /// Retrieve raw chunk bytes
    pub async fn get_chunk(&self, chunk_hash: &str) -> Result<Vec<u8>, ForgeError> {
        let chunk = self
            .store
            .get_chunk(chunk_hash)
            .map_err(|e| ForgeError::Internal(format!("Chunk store error: {}", e)))?
            .ok_or_else(|| ForgeError::NotFound(format!("Chunk {} not found", chunk_hash)))?;
        Ok(chunk)
    }

    /// Retrieve manifest
    pub async fn get_manifest(&self, manifest_id: &str) -> Result<ArtifactManifest, ForgeError> {
        let manifest = self
            .store
            .get_manifest(manifest_id)
            .map_err(|e| ForgeError::Internal(format!("Manifest store error: {}", e)))?
            .ok_or_else(|| ForgeError::NotFound(format!("Manifest {} not found", manifest_id)))?;
        Ok(manifest)
    }

    /// Reconstruct whole object bytes
    pub async fn reconstruct_object(&self, manifest_id: &str) -> Result<Vec<u8>, ForgeError> {
        let manifest = self.get_manifest(manifest_id).await?;
        let mut buffer = Vec::with_capacity(manifest.total_size as usize);

        ObjectReconstructor::reconstruct(
            &manifest,
            |idx| {
                let entry = manifest
                    .chunks
                    .get(idx as usize)
                    .ok_or_else(|| ChunkerError::MissingChunk(idx))?;
                let bytes = self
                    .store
                    .get_chunk(&entry.chunk_hash)
                    .map_err(|e| {
                        ChunkerError::Io(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            e.to_string(),
                        ))
                    })?
                    .ok_or_else(|| ChunkerError::MissingChunk(idx))?;
                Ok(bytes)
            },
            &mut buffer,
        )
        .map_err(|e| ForgeError::Internal(format!("Object reconstruction error: {}", e)))?;

        Ok(buffer)
    }
}
