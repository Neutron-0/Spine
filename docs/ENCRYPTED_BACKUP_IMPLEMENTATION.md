# Authenticated AEAD Encrypted Backup Specification and Implementation

**Milestone:** Spine Batch 20 — Final Deployment Readiness Closure  
**Date:** 2026-11-05  
**Module:** `github-alternative/backend/apps/spine-api/src/backup/mod.rs`  
**Algorithms:** AES-256-GCM (`ring::aead::AES_256_GCM`), PBKDF2-HMAC-SHA256 (`ring::pbkdf2`)

---

## 1. Overview and Security Model

To protect sensitive developer data, native Git repositories, packfiles, release binaries, SSH host credentials, and database snapshots from unauthorized inspection or tampering in untrusted cloud object storage (e.g. S3, GCS, NAS), Spine provides an authenticated, envelope-encrypted backup format (`.spinebak`).

The encryption pipeline guarantees:
1. **Confidentiality:** 256-bit AES-GCM authenticated encryption.
2. **Integrity & Authenticity:** 128-bit authentication tag validating ciphertext and header metadata.
3. **Password Hardening:** PBKDF2 with HMAC-SHA256 and 100,000 iterations using a cryptographically random 16-byte salt generated per archive.
4. **AAD Header Binding:** The entire 42-byte file header (magic, key version, iteration count, salt, nonce) is passed as Additional Authenticated Data (AAD) into AES-GCM. Any tampering with version headers or cryptographic parameters is immediately detected prior to decryption.
5. **Key Versioning & In-Memory Key Rotation:** Archives store a key version identifier and can be re-encrypted to a new passphrase/key without unpacking plain text files to disk.
6. **Path Traversal Sanitization:** Strict path normalization rejecting directory traversal (`..`), absolute roots, Windows drive letters (`C:`), UNC paths, and device names (`CON`, `NUL`, etc.).

---

## 2. Binary Wire Format (`.spinebak`)

A `.spinebak` file consists of a fixed 42-byte header followed by the AES-256-GCM ciphertext and 16-byte authentication tag:

```text
+-----------------------+---------+-------------+-----------+-----------+--------------------+---------------+
| Magic Bytes           | Key Ver | Iterations  | Salt      | Nonce     | Ciphertext Payload | AEAD Auth Tag |
| 'SPNEBAK\x01'         | u16 BE  | u32 BE      | 16 bytes  | 12 bytes  | N bytes            | 16 bytes      |
| Offset 0..7           | 8..9    | 10..13      | 14..29    | 30..41    | 42..(42+N-1)       | (End-16)..End |
+-----------------------+---------+-------------+-----------+-----------+--------------------+---------------+
|<----------------------------- Additional Authenticated Data (AAD) --->|
```

### Header Fields:
- **Magic (8 bytes):** ASCII `SPNEBAK` followed by version byte `0x01` (`0x53, 0x50, 0x4E, 0x45, 0x42, 0x41, 0x4B, 0x01`).
- **Key Version (2 bytes):** Big-endian `u16` identifying the deployment key rotation generation (e.g. `0x0001`, `0x0002`).
- **PBKDF2 Iterations (4 bytes):** Big-endian `u32` specifying the KDF iteration count (default: `100,000`).
- **Salt (16 bytes):** Cryptographically secure pseudorandom bytes generated via `ring::rand::SystemRandom`.
- **Nonce / IV (12 bytes):** Unique 96-bit initialization vector generated per archive.
- **Ciphertext Payload:** The compressed `tar.gz` stream comprising `manifest.json`, `database/snapshot.json`, and all native repository/packfile data.
- **Authentication Tag (16 bytes):** Poly1305 / GHASH tag output by AES-256-GCM.

---

## 3. Cryptographic Lifecycle

### 3.1. Backup Creation (`create_encrypted_backup`)
1. Storage root and database snapshot are packaged into a stream-compressed `.tar.gz` in memory.
2. A 16-byte random salt and 12-byte random nonce are sampled from the OS CSPRNG (`SystemRandom`).
3. PBKDF2-HMAC-SHA256 derives a 32-byte (256-bit) encryption key from the operator passphrase and salt.
4. The 42-byte header is assembled.
5. `ring::aead::AES_256_GCM` seals the buffer in-place with AAD = header, appending the 16-byte authentication tag.
6. The archive is written atomically to the destination path.

### 3.2. Backup Restoration (`restore_encrypted_backup`)
1. The archive is opened and validated: size $\ge 58$ bytes, magic matches `SPNEBAK\x01`.
2. Key version, iteration count, salt, and nonce are extracted from the header.
3. The 32-byte key is derived using PBKDF2 with the extracted salt and provided passphrase.
4. `ring::aead::OpeningKey` decrypts and authenticates the ciphertext in-place against the AAD header.
   - If passphrase is wrong or any bit in the header/ciphertext is modified, AEAD decryption aborts with error.
5. The decrypted payload stream is passed to the tar extraction engine, which performs path traversal sanitization and validates every file against the SHA-256 digests in `manifest.json`.

### 3.3. Zero-Disk Key Rotation (`rotate_backup_key`)
1. Decrypts the payload buffer directly in memory using the existing passphrase.
2. Generates fresh random salt and nonce.
3. Derives new key with the new passphrase.
4. Re-seals the payload with the new key and new key version.
5. Writes the newly encrypted archive. The underlying unencrypted files are never written to disk during rotation.

---

## 4. Path Traversal Hardening

During restore, each relative path in the archive is sanitized via `BackupManager::sanitize_restore_path`:
- Must not be empty.
- Replaces backslashes with standard forward slashes.
- Explicitly rejects paths starting with `/`, `\`, or containing drive letters (`:`).
- Parses path components: rejects `Component::ParentDir` (`..`), `RootDir`, or `Prefix`.
- Rejects Windows reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`).
- Returns the validated, joined destination path strictly rooted inside `target_storage_root`.
