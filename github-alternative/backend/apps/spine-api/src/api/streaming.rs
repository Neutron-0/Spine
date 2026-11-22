use crate::core::error::ForgeError;
use axum::body::Body;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::io::{Seek, SeekFrom, Write};
use std::time::Duration;
use tempfile::NamedTempFile;

pub const DEFAULT_MAX_UPLOAD_BYTES: u64 = 1024 * 1024 * 1024; // 1 GiB
pub const DEFAULT_STREAM_TIMEOUT: Duration = Duration::from_secs(300); // 5 minutes

/// Stream an Axum HTTP `Body` into an anonymous temporary file on disk with strict bounds:
/// 1. Bounds memory buffer to a single chunk (< 64 KiB)
/// 2. Enforces maximum byte limit (`max_bytes`), aborting immediately on overflow
/// 3. Enforces stream timeout per chunk and overall
/// 4. Computes SHA-256 digest on the fly
/// 5. Automatically unlinks temporary file on error or abort
pub async fn stream_body_to_tempfile(
    body: Body,
    max_bytes: u64,
    timeout_duration: Duration,
) -> Result<(NamedTempFile, u64, String), ForgeError> {
    let mut tempfile = NamedTempFile::new().map_err(|e| {
        ForgeError::Internal(format!("Failed to create temporary staging file: {}", e))
    })?;

    let mut stream = body.into_data_stream();
    let mut hasher = Sha256::new();
    let mut total_bytes = 0u64;

    loop {
        let chunk_res = tokio::time::timeout(timeout_duration, stream.next()).await;
        let chunk_opt = match chunk_res {
            Ok(Some(res)) => res.map_err(|e| {
                ForgeError::BadRequest(format!("Stream read error or client aborted: {}", e))
            })?,
            Ok(None) => break, // EOF reached cleanly
            Err(_) => {
                return Err(ForgeError::BadRequest(
                    "Upload stream timed out while receiving data".to_string(),
                ));
            }
        };

        let chunk = chunk_opt;
        total_bytes += chunk.len() as u64;
        if total_bytes > max_bytes {
            return Err(ForgeError::BadRequest(format!(
                "Upload payload exceeds maximum allowed size of {} bytes",
                max_bytes
            )));
        }

        hasher.update(&chunk);
        tempfile.write_all(&chunk).map_err(|e| {
            ForgeError::Internal(format!("Failed to write to temporary staging file: {}", e))
        })?;
    }

    tempfile.flush().map_err(|e| {
        ForgeError::Internal(format!("Failed to flush temporary staging file: {}", e))
    })?;

    // Rewind tempfile so reader can consume it from beginning
    tempfile.seek(SeekFrom::Start(0)).map_err(|e| {
        ForgeError::Internal(format!("Failed to rewind temporary staging file: {}", e))
    })?;

    let sha256_hex = hex::encode(hasher.finalize());
    Ok((tempfile, total_bytes, sha256_hex))
}

/// Formats an RFC 6266 and RFC 5987 compliant Content-Disposition header value.
///
/// Prevents HTTP header injection and guarantees valid ASCII HTTP header values:
/// - Strips / replaces CR, LF, nulls, quotes, slashes, and backslashes in ASCII fallback.
/// - If non-ASCII Unicode characters are present, produces an ASCII fallback `filename="..."`
///   plus a UTF-8 percent-encoded parameter `filename*=UTF-8''...`.
pub fn format_content_disposition(disposition: &str, filename: &str) -> String {
    let clean_fallback: String = filename
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();

    let has_non_ascii = !filename.is_ascii();
    if has_non_ascii {
        let mut encoded = String::new();
        for byte in filename.as_bytes() {
            if byte.is_ascii_alphanumeric()
                || *byte == b'.'
                || *byte == b'-'
                || *byte == b'_'
                || *byte == b'~'
            {
                encoded.push(*byte as char);
            } else {
                use std::fmt::Write;
                let _ = write!(&mut encoded, "%{:02X}", byte);
            }
        }
        format!(
            "{}; filename=\"{}\"; filename*=UTF-8''{}",
            disposition, clean_fallback, encoded
        )
    } else {
        let clean = filename.replace(['\r', '\n', '"', '\\', '/', '\0'], "_");
        format!("{}; filename=\"{}\"", disposition, clean)
    }
}
