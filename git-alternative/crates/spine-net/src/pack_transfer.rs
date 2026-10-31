//! Packfile streaming, chunking, and atomic receipt over Spine Wire Protocol.

use spine_core::ObjectId;
use spine_object_store::{
    pack::PackWriter, pack_index::PackIndexWriter, CompositeObjectStore, ObjectStore, PackReader,
};
use spine_protocol::{
    frame::{Frame, FrameType, ProtocolError},
    messages::{PackEnd, PackProgress, ProtocolMessage},
};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use uuid::Uuid;

pub const PACK_CHUNK_SIZE: usize = 64 * 1024; // 64 KB

#[derive(Error, Debug)]
pub enum TransferError {
    #[error("Object store error: {0}")]
    Store(#[from] spine_object_store::ObjectStoreError),
    #[error("Pack error: {0}")]
    Pack(#[from] spine_object_store::pack::PackError),
    #[error("Pack index error: {0}")]
    PackIndex(#[from] spine_object_store::pack_index::PackIndexError),
    #[error("Protocol error: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Cumulative checksum mismatch on received pack")]
    ChecksumMismatch,
    #[error("Pack transmission aborted prematurely")]
    Aborted,
}

/// Creates a self-contained `.spk` packfile containing `objects` from `store`.
pub fn create_pack_file(
    store: &CompositeObjectStore,
    objects: &[ObjectId],
    output_path: &Path,
) -> Result<([u8; 32], u32), TransferError> {
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let count = objects.len() as u32;
    let mut writer = PackWriter::new(output_path, store.algorithm(), count, 3)?;

    for oid in objects {
        let (obj_type, payload) = store.read_object(oid)?;
        writer.append_object(oid, obj_type, &payload)?;
    }

    let trailer_hash = writer.finish()?;
    let mut full_hash = [0u8; 32];
    full_hash[0..28].copy_from_slice(&trailer_hash);

    Ok((full_hash, count))
}

/// Streams a `.spk` packfile across an async writer as `PACK_DATA` frames.
pub async fn send_pack_file<W: AsyncWrite + Unpin>(
    pack_path: &Path,
    checksum: [u8; 32],
    object_count: u32,
    mut writer: W,
) -> Result<(), TransferError> {
    let mut file = File::open(pack_path)?;
    let total_bytes = file.metadata()?.len();
    let mut buffer = vec![0u8; PACK_CHUNK_SIZE];
    let mut sent_bytes = 0u64;

    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }

        sent_bytes += n as u64;
        let chunk = buffer[..n].to_vec();
        let frame = Frame::new(FrameType::PackData, chunk)?;
        frame.write_to_async(&mut writer).await?;

        // Emit progress frame occasionally
        if sent_bytes % (512 * 1024) < (PACK_CHUNK_SIZE as u64) || sent_bytes == total_bytes {
            let progress = PackProgress {
                phase: "streaming".to_string(),
                current: sent_bytes,
                total: total_bytes,
                message: format!("Transferred {} / {} bytes", sent_bytes, total_bytes),
            };
            let prog_frame = progress.to_frame()?;
            prog_frame.write_to_async(&mut writer).await?;
        }
    }

    // Send final PackEnd frame
    let end_msg = PackEnd {
        pack_checksum: checksum,
        object_count,
    };
    let end_frame = end_msg.to_frame()?;
    end_frame.write_to_async(&mut writer).await?;

    Ok(())
}

/// Receives a packfile stream from an async reader, validates it, indexes it,
/// and atomically commits it into `store`.
pub async fn receive_and_ingest_pack<R: AsyncRead + Unpin>(
    mut reader: R,
    spine_dir: &Path,
    store: &CompositeObjectStore,
) -> Result<Option<String>, TransferError> {
    let pack_dir = spine_dir.join("objects").join("pack");
    fs::create_dir_all(&pack_dir)?;

    let temp_pack_name = format!(".incoming-{}.spk", Uuid::new_v4());
    let temp_pack_path = pack_dir.join(&temp_pack_name);

    let mut temp_file = File::create(&temp_pack_path)?;
    let mut received_bytes = 0u64;
    let mut expected_end: Option<PackEnd> = None;

    let receive_res: Result<(), TransferError> = async {
        loop {
            let frame = Frame::read_from_async(&mut reader).await?;
            match frame.header.frame_type {
                FrameType::PackData => {
                    temp_file.write_all(&frame.payload)?;
                    received_bytes += frame.payload.len() as u64;
                }
                FrameType::PackProgress => {
                    // Diagnostic feedback ignored on server ingestion
                }
                FrameType::PackEnd => {
                    let end_msg = PackEnd::from_frame(&frame)?;
                    expected_end = Some(end_msg);
                    break;
                }
                FrameType::Error => {
                    return Err(TransferError::Aborted);
                }
                _ => {}
            }
        }
        Ok(())
    }
    .await;

    if let Err(e) = receive_res {
        let _ = fs::remove_file(&temp_pack_path);
        return Err(e);
    }

    let end_msg = match expected_end {
        Some(e) => e,
        None => {
            let _ = fs::remove_file(&temp_pack_path);
            return Err(TransferError::Aborted);
        }
    };

    if end_msg.object_count == 0 || received_bytes == 0 {
        let _ = fs::remove_file(&temp_pack_path);
        return Ok(None);
    }

    temp_file.flush()?;
    drop(temp_file);

    // Validate pack and scan index entries
    let (pack_id, entries) = match (|| -> Result<(String, _), TransferError> {
        let mut pack_reader = PackReader::open(&temp_pack_path)?;
        let entries = pack_reader.scan_index_entries()?;
        let pack_id = format!("pack-{}", Uuid::new_v4());
        Ok((pack_id, entries))
    })() {
        Ok(res) => res,
        Err(e) => {
            let _ = fs::remove_file(&temp_pack_path);
            return Err(e);
        }
    };

    // Write .spidx index
    let temp_idx_path = pack_dir.join(format!("{}.spidx", pack_id));
    if let Err(e) = PackIndexWriter::write_index(&temp_idx_path, store.algorithm(), entries) {
        let _ = fs::remove_file(&temp_pack_path);
        let _ = fs::remove_file(&temp_idx_path);
        return Err(TransferError::PackIndex(e));
    }

    // Atomically move pack into final location
    let final_pack_path = pack_dir.join(format!("{}.spk", pack_id));
    if let Err(e) = spine_core::atomic_replace_file(&temp_pack_path, &final_pack_path) {
        let _ = fs::remove_file(&temp_pack_path);
        let _ = fs::remove_file(&temp_idx_path);
        return Err(TransferError::Io(e));
    }

    // Register pack in catalog and hot-reload readers
    let pack_size = fs::metadata(&final_pack_path)?.len();
    let idx_size = fs::metadata(&temp_idx_path)?.len();

    store.register_pack(
        pack_id.clone(),
        end_msg.object_count,
        pack_size,
        idx_size,
        end_msg.pack_checksum,
    )?;

    Ok(Some(pack_id))
}
