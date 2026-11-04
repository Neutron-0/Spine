//! Unified distributed artifact storage, chunk manifests, resumable streaming,
//! Large Object Storage (Spine LFS), and garbage collection.

pub mod chunker;
pub mod gc;
pub mod lfs;
pub mod manifest;
pub mod store;
pub mod transfer;

pub use chunker::{ChunkData, ChunkingResult, ObjectReconstructor, StreamChunker};
pub use gc::{ArtifactGcCollector, ArtifactGcReport};
pub use lfs::{LfsPointer, LfsTrackingRules};
pub use manifest::{ArtifactManifest, ChunkEntry, DEFAULT_CHUNK_SIZE, MANIFEST_VERSION};
pub use store::{ArtifactStore, DistributedArtifactStore, LocalArtifactStore, StoreError};
pub use transfer::{TransferError, TransferSession, TransferState};
