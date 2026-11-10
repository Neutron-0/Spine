use crate::core::error::ForgeError;
use ring::aead::{
    Aad, BoundKey, Nonce, NonceSequence, OpeningKey, SealingKey, UnboundKey, AES_256_GCM, NONCE_LEN,
};
use ring::pbkdf2;
use ring::rand::{SecureRandom, SystemRandom};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::num::NonZeroU32;
use uuid::Uuid;

pub const MAGIC_V2: &[u8; 8] = b"SPNEBAK\x02";
pub const DEFAULT_CHUNK_SIZE: u32 = 1024 * 1024; // 1 MiB bounded streaming buffer
pub const DEFAULT_PBKDF2_ITERATIONS: u32 = 100_000;
pub const SALT_LEN: usize = 16;
pub const NONCE_PREFIX_LEN: usize = 4;
pub const HEADER_V2_LEN: usize = 8 + 2 + 4 + SALT_LEN + 16 + NONCE_PREFIX_LEN + 4; // 54 bytes
pub const TAG_LEN: usize = 16;

pub const CHUNK_TYPE_DATA: u8 = 0x01;
pub const CHUNK_TYPE_FINAL: u8 = 0x02;

struct OneNonce(Option<[u8; NONCE_LEN]>);

impl NonceSequence for OneNonce {
    fn advance(&mut self) -> Result<Nonce, ring::error::Unspecified> {
        self.0
            .take()
            .map(Nonce::assume_unique_for_key)
            .ok_or(ring::error::Unspecified)
    }
}

/// Header metadata for SPNEBAK v2 streaming format.
#[derive(Debug, Clone)]
pub struct BackupHeaderV2 {
    pub key_version: u16,
    pub pbkdf2_iterations: u32,
    pub salt: [u8; SALT_LEN],
    pub archive_id: [u8; 16],
    pub nonce_prefix: [u8; NONCE_PREFIX_LEN],
    pub chunk_size: u32,
}

impl BackupHeaderV2 {
    pub fn to_bytes(&self) -> [u8; HEADER_V2_LEN] {
        let mut buf = [0u8; HEADER_V2_LEN];
        buf[0..8].copy_from_slice(MAGIC_V2);
        buf[8..10].copy_from_slice(&self.key_version.to_be_bytes());
        buf[10..14].copy_from_slice(&self.pbkdf2_iterations.to_be_bytes());
        buf[14..30].copy_from_slice(&self.salt);
        buf[30..46].copy_from_slice(&self.archive_id);
        buf[46..50].copy_from_slice(&self.nonce_prefix);
        buf[50..54].copy_from_slice(&self.chunk_size.to_be_bytes());
        buf
    }

    pub fn read_from<R: std::io::Read>(mut reader: R) -> Result<Self, ForgeError> {
        let mut buf = [0u8; HEADER_V2_LEN];
        reader
            .read_exact(&mut buf)
            .map_err(|e| ForgeError::Internal(format!("Failed to read backup header: {}", e)))?;
        Self::from_bytes(&buf)
    }

    pub fn from_bytes(buf: &[u8; HEADER_V2_LEN]) -> Result<Self, ForgeError> {
        if &buf[0..8] != MAGIC_V2 {
            return Err(ForgeError::Internal(
                "Invalid backup header: magic mismatch".to_string(),
            ));
        }
        let key_version = u16::from_be_bytes([buf[8], buf[9]]);
        let pbkdf2_iterations = u32::from_be_bytes([buf[10], buf[11], buf[12], buf[13]]);
        let mut salt = [0u8; SALT_LEN];
        salt.copy_from_slice(&buf[14..30]);
        let mut archive_id = [0u8; 16];
        archive_id.copy_from_slice(&buf[30..46]);
        let mut nonce_prefix = [0u8; NONCE_PREFIX_LEN];
        nonce_prefix.copy_from_slice(&buf[46..50]);
        let chunk_size = u32::from_be_bytes([buf[50], buf[51], buf[52], buf[53]]);

        if !(10_000..=1_000_000).contains(&pbkdf2_iterations) {
            return Err(ForgeError::Internal(format!(
                "Invalid PBKDF2 iterations in backup header: {} (must be between 10,000 and 1,000,000)",
                pbkdf2_iterations
            )));
        }

        if !(4096..=16 * 1024 * 1024).contains(&chunk_size) {
            return Err(ForgeError::Internal(format!(
                "Invalid chunk size in backup header: {} bytes (must be between 4096 and 16,777,216 bytes)",
                chunk_size
            )));
        }

        Ok(Self {
            key_version,
            pbkdf2_iterations,
            salt,
            archive_id,
            nonce_prefix,
            chunk_size,
        })
    }
}

/// Summary report returned after streaming encryption or decryption.
#[derive(Debug, Clone)]
pub struct StreamSummary {
    pub total_chunks: u64,
    pub total_plaintext_bytes: u64,
    pub stream_sha256: [u8; 32],
}

/// Derive a 256-bit AES-GCM key using PBKDF2-HMAC-SHA256.
pub fn derive_key(passphrase: &str, salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut key = [0u8; 32];
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        NonZeroU32::new(iterations).unwrap_or(NonZeroU32::new(DEFAULT_PBKDF2_ITERATIONS).unwrap()),
        salt,
        passphrase.as_bytes(),
        &mut key,
    );
    key
}

/// Construct a unique 12-byte AES-GCM nonce: [4 bytes nonce_prefix][8 bytes chunk_index BE].
pub fn build_chunk_nonce(
    nonce_prefix: &[u8; NONCE_PREFIX_LEN],
    chunk_index: u64,
) -> [u8; NONCE_LEN] {
    let mut nonce = [0u8; NONCE_LEN];
    nonce[0..4].copy_from_slice(nonce_prefix);
    nonce[4..12].copy_from_slice(&chunk_index.to_be_bytes());
    nonce
}

/// Construct Authenticated Additional Data (AAD) for a chunk.
/// Binds: Archive ID (16B) || Chunk Index (8B BE) || Chunk Type (1B) || Plaintext Length (4B BE).
pub fn build_chunk_aad(
    archive_id: &[u8; 16],
    chunk_index: u64,
    chunk_type: u8,
    plaintext_len: u32,
) -> [u8; 29] {
    let mut aad = [0u8; 29];
    aad[0..16].copy_from_slice(archive_id);
    aad[16..24].copy_from_slice(&chunk_index.to_be_bytes());
    aad[24] = chunk_type;
    aad[25..29].copy_from_slice(&plaintext_len.to_be_bytes());
    aad
}

/// Streaming authenticated encrypting writer (SPNEBAK v2).
/// Wraps any `Write` sink, derives keys, writes header, and seals plaintext in bounded chunks.
/// Enables streaming TAR and GZ generation directly into AEAD ciphertext without temporary disk staging.
pub struct EncryptingWriter<W: Write> {
    writer: W,
    key_bytes: [u8; 32],
    archive_id: [u8; 16],
    nonce_prefix: [u8; NONCE_PREFIX_LEN],
    chunk_size: usize,
    buffer: Vec<u8>,
    chunk_index: u64,
    total_bytes: u64,
    stream_hasher: Sha256,
    finished: bool,
}

impl<W: Write> EncryptingWriter<W> {
    pub fn new(
        mut writer: W,
        passphrase: &str,
        key_version: u16,
        chunk_size: u32,
    ) -> Result<Self, ForgeError> {
        if passphrase.trim().is_empty() {
            return Err(ForgeError::Internal(
                "Backup encryption passphrase cannot be empty".to_string(),
            ));
        }

        let rng = SystemRandom::new();
        let mut salt = [0u8; SALT_LEN];
        rng.fill(&mut salt)
            .map_err(|_| ForgeError::Internal("Failed to generate random salt".to_string()))?;

        let mut nonce_prefix = [0u8; NONCE_PREFIX_LEN];
        rng.fill(&mut nonce_prefix).map_err(|_| {
            ForgeError::Internal("Failed to generate random nonce prefix".to_string())
        })?;

        let archive_uuid = Uuid::new_v4();
        let archive_id = *archive_uuid.as_bytes();

        let header = BackupHeaderV2 {
            key_version,
            pbkdf2_iterations: DEFAULT_PBKDF2_ITERATIONS,
            salt,
            archive_id,
            nonce_prefix,
            chunk_size,
        };

        writer
            .write_all(&header.to_bytes())
            .map_err(|e| ForgeError::Internal(format!("Failed to write header: {}", e)))?;

        let key_bytes = derive_key(passphrase, &salt, DEFAULT_PBKDF2_ITERATIONS);

        Ok(Self {
            writer,
            key_bytes,
            archive_id,
            nonce_prefix,
            chunk_size: chunk_size as usize,
            buffer: Vec::with_capacity(chunk_size as usize),
            chunk_index: 0,
            total_bytes: 0,
            stream_hasher: Sha256::new(),
            finished: false,
        })
    }

    fn write_chunk(&mut self, data: &[u8]) -> Result<(), ForgeError> {
        self.total_bytes += data.len() as u64;
        self.stream_hasher.update(data);

        let nonce = build_chunk_nonce(&self.nonce_prefix, self.chunk_index);
        let aad = build_chunk_aad(
            &self.archive_id,
            self.chunk_index,
            CHUNK_TYPE_DATA,
            data.len() as u32,
        );

        let unbound_key = UnboundKey::new(&AES_256_GCM, &self.key_bytes)
            .map_err(|e| ForgeError::Internal(format!("AEAD key error: {:?}", e)))?;
        let mut sealing_key = SealingKey::new(unbound_key, OneNonce(Some(nonce)));

        let mut payload = data.to_vec();
        sealing_key
            .seal_in_place_append_tag(Aad::from(&aad), &mut payload)
            .map_err(|e| ForgeError::Internal(format!("AEAD seal error: {:?}", e)))?;

        let ct_len = payload.len() as u32;
        self.writer
            .write_all(&[CHUNK_TYPE_DATA])
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        self.writer
            .write_all(&self.chunk_index.to_be_bytes())
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        self.writer
            .write_all(&ct_len.to_be_bytes())
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        self.writer
            .write_all(&payload)
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        self.chunk_index += 1;
        Ok(())
    }

    pub fn finish(mut self) -> Result<StreamSummary, ForgeError> {
        if self.finished {
            return Err(ForgeError::Internal(
                "EncryptingWriter already finished".into(),
            ));
        }
        self.finished = true;

        if !self.buffer.is_empty() {
            let data = std::mem::take(&mut self.buffer);
            self.write_chunk(&data)?;
        }

        // Write final authenticated manifest chunk (CHUNK_TYPE_FINAL)
        let stream_hash: [u8; 32] = self.stream_hasher.finalize().into();
        let mut manifest_payload = Vec::with_capacity(8 + 8 + 32);
        manifest_payload.extend_from_slice(&self.chunk_index.to_be_bytes()); // total data chunks
        manifest_payload.extend_from_slice(&self.total_bytes.to_be_bytes()); // total bytes
        manifest_payload.extend_from_slice(&stream_hash);

        let manifest_plain_len = manifest_payload.len() as u32;
        let nonce = build_chunk_nonce(&self.nonce_prefix, self.chunk_index);
        let aad = build_chunk_aad(
            &self.archive_id,
            self.chunk_index,
            CHUNK_TYPE_FINAL,
            manifest_plain_len,
        );

        let unbound_key = UnboundKey::new(&AES_256_GCM, &self.key_bytes)
            .map_err(|e| ForgeError::Internal(format!("AEAD key error: {:?}", e)))?;
        let mut sealing_key = SealingKey::new(unbound_key, OneNonce(Some(nonce)));

        sealing_key
            .seal_in_place_append_tag(Aad::from(&aad), &mut manifest_payload)
            .map_err(|e| {
                ForgeError::Internal(format!("AEAD seal error on final manifest: {:?}", e))
            })?;

        let final_ct_len = manifest_payload.len() as u32;
        self.writer
            .write_all(&[CHUNK_TYPE_FINAL])
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        self.writer
            .write_all(&self.chunk_index.to_be_bytes())
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        self.writer
            .write_all(&final_ct_len.to_be_bytes())
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        self.writer
            .write_all(&manifest_payload)
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        self.writer
            .flush()
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        Ok(StreamSummary {
            total_chunks: self.chunk_index,
            total_plaintext_bytes: self.total_bytes,
            stream_sha256: stream_hash,
        })
    }
}

impl<W: Write> Write for EncryptingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buffer.extend_from_slice(buf);
        while self.buffer.len() >= self.chunk_size {
            let chunk: Vec<u8> = self.buffer.drain(..self.chunk_size).collect();
            self.write_chunk(&chunk)
                .map_err(|e| std::io::Error::other(e.to_string()))?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}

/// Streams plaintext from `reader`, encrypts in bounded chunks using AES-256-GCM,
/// and writes the SPNEBAK v2 framed archive to `writer`.
///
/// Memory footprint is strictly bounded to O(chunk_size) regardless of stream length.
pub fn encrypt_stream<R: Read, W: Write>(
    mut reader: R,
    writer: W,
    passphrase: &str,
    key_version: u16,
    chunk_size: u32,
) -> Result<StreamSummary, ForgeError> {
    let mut enc_writer = EncryptingWriter::new(writer, passphrase, key_version, chunk_size)?;
    let mut buf = vec![0u8; chunk_size as usize];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| ForgeError::Internal(format!("Read error: {}", e)))?;
        if n == 0 {
            break;
        }
        enc_writer
            .write_all(&buf[..n])
            .map_err(|e| ForgeError::Internal(format!("Write error: {}", e)))?;
    }
    enc_writer.finish()
}

/// Streams encrypted chunks from `reader`, authenticates and decrypts with AES-256-GCM,
/// and writes verified plaintext to `writer`.
///
/// Memory footprint is strictly bounded to O(chunk_size).
pub fn decrypt_stream<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    passphrase: &str,
) -> Result<StreamSummary, ForgeError> {
    let mut header_buf = [0u8; HEADER_V2_LEN];
    reader
        .read_exact(&mut header_buf)
        .map_err(|e| ForgeError::Internal(format!("Failed to read backup header: {}", e)))?;

    let header = BackupHeaderV2::from_bytes(&header_buf)?;
    let key_bytes = derive_key(passphrase, &header.salt, header.pbkdf2_iterations);

    let mut expected_chunk_index = 0u64;
    let mut total_bytes = 0u64;
    let mut stream_hasher = Sha256::new();

    loop {
        let mut type_buf = [0u8; 1];
        match reader.read_exact(&mut type_buf) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Err(ForgeError::Internal(
                    "Unexpected EOF: missing final authenticated manifest chunk".to_string(),
                ));
            }
            Err(e) => return Err(ForgeError::Internal(format!("Read error: {}", e))),
        }

        let chunk_type = type_buf[0];

        let mut idx_buf = [0u8; 8];
        reader
            .read_exact(&mut idx_buf)
            .map_err(|e| ForgeError::Internal(format!("Failed to read chunk index: {}", e)))?;
        let chunk_index = u64::from_be_bytes(idx_buf);

        if chunk_index != expected_chunk_index {
            return Err(ForgeError::Internal(format!(
                "Chunk sequencing violation: expected index {}, got {}",
                expected_chunk_index, chunk_index
            )));
        }

        let mut len_buf = [0u8; 4];
        reader
            .read_exact(&mut len_buf)
            .map_err(|e| ForgeError::Internal(format!("Failed to read chunk length: {}", e)))?;
        let ct_len = u32::from_be_bytes(len_buf) as usize;

        if ct_len < TAG_LEN || ct_len > (header.chunk_size as usize + TAG_LEN + 1024) {
            return Err(ForgeError::Internal(format!(
                "Invalid chunk ciphertext length: {} bytes",
                ct_len
            )));
        }

        let mut payload = vec![0u8; ct_len];
        reader
            .read_exact(&mut payload)
            .map_err(|e| ForgeError::Internal(format!("Failed to read chunk payload: {}", e)))?;

        let plain_len = (ct_len - TAG_LEN) as u32;
        let nonce = build_chunk_nonce(&header.nonce_prefix, chunk_index);
        let aad = build_chunk_aad(&header.archive_id, chunk_index, chunk_type, plain_len);

        let unbound_key = UnboundKey::new(&AES_256_GCM, &key_bytes)
            .map_err(|e| ForgeError::Internal(format!("AEAD key error: {:?}", e)))?;
        let mut opening_key = OpeningKey::new(unbound_key, OneNonce(Some(nonce)));

        let decrypted = opening_key
            .open_in_place(Aad::from(&aad), &mut payload)
            .map_err(|_| {
                ForgeError::Internal(format!(
                    "Authentication failure: invalid passphrase or corrupted chunk at index {}",
                    chunk_index
                ))
            })?;

        if chunk_type == CHUNK_TYPE_FINAL {
            // Verify final manifest
            if decrypted.len() != (8 + 8 + 32) {
                return Err(ForgeError::Internal(
                    "Corrupt final manifest chunk: invalid size".to_string(),
                ));
            }
            let reported_chunks = u64::from_be_bytes(decrypted[0..8].try_into().unwrap());
            let reported_bytes = u64::from_be_bytes(decrypted[8..16].try_into().unwrap());
            let reported_hash: [u8; 32] = decrypted[16..48].try_into().unwrap();

            if reported_chunks != expected_chunk_index {
                return Err(ForgeError::Internal(format!(
                    "Manifest validation error: chunk count mismatch (reported: {}, actual: {})",
                    reported_chunks, expected_chunk_index
                )));
            }

            if reported_bytes != total_bytes {
                return Err(ForgeError::Internal(format!(
                    "Manifest validation error: byte count mismatch (reported: {}, actual: {})",
                    reported_bytes, total_bytes
                )));
            }

            let actual_hash: [u8; 32] = stream_hasher.finalize().into();
            if actual_hash != reported_hash {
                return Err(ForgeError::Internal(
                    "Manifest validation error: overall stream SHA-256 hash mismatch".to_string(),
                ));
            }

            writer
                .flush()
                .map_err(|e| ForgeError::Internal(format!("Flush error: {}", e)))?;

            // Security check: enforce strictly zero trailing bytes after final manifest
            let mut trailing_buf = [0u8; 1];
            match reader.read(&mut trailing_buf) {
                Ok(0) => {} // Clean EOF
                Ok(_) => {
                    return Err(ForgeError::Internal(
                        "Security violation: unexpected trailing data after final manifest chunk"
                            .to_string(),
                    ));
                }
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {}
                Err(e) => {
                    return Err(ForgeError::Internal(format!(
                        "Error checking for trailing data: {}",
                        e
                    )));
                }
            }

            return Ok(StreamSummary {
                total_chunks: reported_chunks,
                total_plaintext_bytes: total_bytes,
                stream_sha256: actual_hash,
            });
        } else if chunk_type == CHUNK_TYPE_DATA {
            total_bytes += decrypted.len() as u64;
            stream_hasher.update(&*decrypted);
            writer
                .write_all(decrypted)
                .map_err(|e| ForgeError::Internal(format!("Write error: {}", e)))?;
            expected_chunk_index += 1;
        } else {
            return Err(ForgeError::Internal(format!(
                "Unknown chunk type: {}",
                chunk_type
            )));
        }
    }
}

/// Rotates encryption key/passphrase streaming directly from `reader` to `writer`,
/// decrypting and re-encrypting chunk by chunk without writing unencrypted data to disk.
///
/// Memory footprint is strictly bounded to O(chunk_size).
pub fn rotate_stream<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    old_passphrase: &str,
    new_passphrase: &str,
    new_key_version: u16,
) -> Result<(), ForgeError> {
    if new_passphrase.trim().is_empty() {
        return Err(ForgeError::Internal(
            "New backup encryption passphrase cannot be empty".to_string(),
        ));
    }

    let mut header_buf = [0u8; HEADER_V2_LEN];
    reader
        .read_exact(&mut header_buf)
        .map_err(|e| ForgeError::Internal(format!("Failed to read source header: {}", e)))?;

    let old_header = BackupHeaderV2::from_bytes(&header_buf)?;
    let old_key = derive_key(
        old_passphrase,
        &old_header.salt,
        old_header.pbkdf2_iterations,
    );

    let rng = SystemRandom::new();
    let mut new_salt = [0u8; SALT_LEN];
    rng.fill(&mut new_salt)
        .map_err(|_| ForgeError::Internal("Failed to generate random salt".to_string()))?;

    let mut new_nonce_prefix = [0u8; NONCE_PREFIX_LEN];
    rng.fill(&mut new_nonce_prefix)
        .map_err(|_| ForgeError::Internal("Failed to generate random nonce prefix".to_string()))?;

    let new_archive_uuid = Uuid::new_v4();
    let new_archive_id = *new_archive_uuid.as_bytes();

    let new_header = BackupHeaderV2 {
        key_version: new_key_version,
        pbkdf2_iterations: DEFAULT_PBKDF2_ITERATIONS,
        salt: new_salt,
        archive_id: new_archive_id,
        nonce_prefix: new_nonce_prefix,
        chunk_size: old_header.chunk_size,
    };

    writer
        .write_all(&new_header.to_bytes())
        .map_err(|e| ForgeError::Internal(format!("Failed to write new header: {}", e)))?;

    let new_key = derive_key(new_passphrase, &new_salt, DEFAULT_PBKDF2_ITERATIONS);

    let mut expected_chunk_index = 0u64;
    let mut total_bytes = 0u64;
    let mut stream_hasher = Sha256::new();

    loop {
        let mut type_buf = [0u8; 1];
        match reader.read_exact(&mut type_buf) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return Err(ForgeError::Internal(
                    "Unexpected EOF during key rotation: missing final manifest chunk".to_string(),
                ));
            }
            Err(e) => return Err(ForgeError::Internal(format!("Read error: {}", e))),
        }

        let chunk_type = type_buf[0];

        let mut idx_buf = [0u8; 8];
        reader
            .read_exact(&mut idx_buf)
            .map_err(|e| ForgeError::Internal(format!("Failed to read chunk index: {}", e)))?;
        let chunk_index = u64::from_be_bytes(idx_buf);

        if chunk_index != expected_chunk_index {
            return Err(ForgeError::Internal(format!(
                "Chunk sequencing violation during rotation: expected {}, got {}",
                expected_chunk_index, chunk_index
            )));
        }

        let mut len_buf = [0u8; 4];
        reader
            .read_exact(&mut len_buf)
            .map_err(|e| ForgeError::Internal(format!("Failed to read chunk length: {}", e)))?;
        let ct_len = u32::from_be_bytes(len_buf) as usize;

        if ct_len < TAG_LEN || ct_len > (old_header.chunk_size as usize + TAG_LEN + 1024) {
            return Err(ForgeError::Internal(format!(
                "Invalid chunk ciphertext length in rotation: {} bytes (exceeds chunk bound)",
                ct_len
            )));
        }

        let mut payload = vec![0u8; ct_len];
        reader
            .read_exact(&mut payload)
            .map_err(|e| ForgeError::Internal(format!("Failed to read payload: {}", e)))?;

        let plain_len = (ct_len - TAG_LEN) as u32;

        // 1. Decrypt chunk with old key
        let old_nonce = build_chunk_nonce(&old_header.nonce_prefix, chunk_index);
        let old_aad = build_chunk_aad(&old_header.archive_id, chunk_index, chunk_type, plain_len);

        let old_unbound = UnboundKey::new(&AES_256_GCM, &old_key)
            .map_err(|e| ForgeError::Internal(format!("AEAD key error: {:?}", e)))?;
        let mut old_opening = OpeningKey::new(old_unbound, OneNonce(Some(old_nonce)));

        let decrypted = old_opening
            .open_in_place(Aad::from(&old_aad), &mut payload)
            .map_err(|_| {
                ForgeError::Internal(format!(
                    "Decryption failure on chunk index {} during rotation (invalid passphrase?)",
                    chunk_index
                ))
            })?;

        // 2. Re-encrypt chunk with new key
        let new_nonce = build_chunk_nonce(&new_nonce_prefix, chunk_index);
        let new_aad = build_chunk_aad(&new_archive_id, chunk_index, chunk_type, plain_len);

        let new_unbound = UnboundKey::new(&AES_256_GCM, &new_key)
            .map_err(|e| ForgeError::Internal(format!("AEAD key error: {:?}", e)))?;
        let mut new_sealing = SealingKey::new(new_unbound, OneNonce(Some(new_nonce)));

        let mut new_payload = decrypted.to_vec();
        new_sealing
            .seal_in_place_append_tag(Aad::from(&new_aad), &mut new_payload)
            .map_err(|e| ForgeError::Internal(format!("Re-encryption error: {:?}", e)))?;

        let new_ct_len = new_payload.len() as u32;
        writer
            .write_all(&[chunk_type])
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        writer
            .write_all(&chunk_index.to_be_bytes())
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        writer
            .write_all(&new_ct_len.to_be_bytes())
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        writer
            .write_all(&new_payload)
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        if chunk_type == CHUNK_TYPE_FINAL {
            // Verify final manifest contents
            if decrypted.len() != (8 + 8 + 32) {
                return Err(ForgeError::Internal(
                    "Corrupt final manifest chunk in rotation: invalid size".to_string(),
                ));
            }
            let reported_chunks = u64::from_be_bytes(decrypted[0..8].try_into().unwrap());
            let reported_bytes = u64::from_be_bytes(decrypted[8..16].try_into().unwrap());
            let reported_hash: [u8; 32] = decrypted[16..48].try_into().unwrap();

            if reported_chunks != expected_chunk_index {
                return Err(ForgeError::Internal(format!(
                    "Rotation manifest validation error: chunk count mismatch (reported: {}, actual: {})",
                    reported_chunks, expected_chunk_index
                )));
            }

            if reported_bytes != total_bytes {
                return Err(ForgeError::Internal(format!(
                    "Rotation manifest validation error: byte count mismatch (reported: {}, actual: {})",
                    reported_bytes, total_bytes
                )));
            }

            let actual_hash: [u8; 32] = stream_hasher.finalize().into();
            if actual_hash != reported_hash {
                return Err(ForgeError::Internal(
                    "Rotation manifest validation error: stream SHA-256 hash mismatch".to_string(),
                ));
            }

            writer
                .flush()
                .map_err(|e| ForgeError::Internal(format!("Flush error: {}", e)))?;

            // Check for trailing bytes
            let mut trailing_buf = [0u8; 1];
            match reader.read(&mut trailing_buf) {
                Ok(0) => {} // Clean EOF
                Ok(_) => {
                    return Err(ForgeError::Internal(
                        "Security violation: unexpected trailing data after final manifest chunk in rotation".to_string(),
                    ));
                }
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {}
                Err(e) => {
                    return Err(ForgeError::Internal(format!(
                        "Error checking for trailing data: {}",
                        e
                    )));
                }
            }

            return Ok(());
        } else if chunk_type == CHUNK_TYPE_DATA {
            total_bytes += decrypted.len() as u64;
            stream_hasher.update(&*decrypted);
            expected_chunk_index += 1;
        } else {
            return Err(ForgeError::Internal(format!(
                "Unknown chunk type during rotation: {}",
                chunk_type
            )));
        }
    }
}
