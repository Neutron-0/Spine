# SPNEBAK v2 — Streaming Chunked Authenticated Backup Format

**Identifier:** `SPNEBAK\x02`  
**Cipher Suite:** AES-256-GCM (`AEAD_AES_256_GCM`)  
**KDF:** PBKDF2-HMAC-SHA256 (100,000 iterations)  
**Memory Complexity:** $O(\text{chunk\_size})$ (Bounded 1 MiB RAM)  
**Maximum Supported Archive:** $>64\text{ GiB}$ without integer or nonce overflow

---

## 1. Motivation & Threat Model

In Batch 20, backup encryption (SPNEBAK v1) utilized a single-invocation AES-256-GCM AEAD pass over the entire uncompressed/compressed tar archive. For production deployments with tens of gigabytes of repository history, this caused unbounded memory consumption and crashes (Finding F-02).

SPNEBAK v2 solves this with:
1. **Streaming Processing:** Divides the archive stream into bounded chunks (default 1 MiB plaintext per chunk).
2. **Unique Per-Chunk Nonces:** Prevents nonce reuse across chunks and archives.
3. **Chunk Tampering & Reordering Defense:** Authenticated Additional Data (AAD) cryptographically binds the archive UUID, sequence index, chunk type, and chunk length.
4. **Final Authenticated Manifest:** Terminal chunk binds total chunks, total plaintext bytes, and full-stream SHA-256 hash.
5. **Streaming Key Rotation:** Re-keys an archive directly from reader to writer without decrypting to disk.

---

## 2. Binary Layout

```
+---------------------------------------------------------------+
| HEADER (54 Bytes)                                             |
|  - Magic: "SPNEBAK\x02" (8 Bytes)                             |
|  - Key Version: u16 BE (2 Bytes)                              |
|  - PBKDF2 Iterations: u32 BE (4 Bytes)                        |
|  - Salt: [u8; 16] (16 Bytes)                                  |
|  - Archive ID: [u8; 16] (UUID v4)                             |
|  - Nonce Prefix: [u8; 4] (CSPRNG random)                      |
|  - Chunk Size: u32 BE (4 Bytes, default 1,048,576)            |
+---------------------------------------------------------------+
| REPEATED DATA CHUNKS                                          |
|  - Chunk Type: 0x01 (1 Byte)                                  |
|  - Chunk Index: u64 BE (8 Bytes)                              |
|  - Ciphertext Length: u32 BE (4 Bytes = Plain Len + 16 Tag)   |
|  - Ciphertext + Tag: [u8; Ciphertext Length]                  |
+---------------------------------------------------------------+
| FINAL MANIFEST CHUNK                                          |
|  - Chunk Type: 0x02 (1 Byte)                                  |
|  - Chunk Index: u64 BE (8 Bytes = total_chunks)               |
|  - Ciphertext Length: u32 BE (4 Bytes)                        |
|  - Ciphertext + Tag: [u8; Ciphertext Length]                  |
|    Decrypted Payload contains:                                |
|      * Total Chunks: u64 BE                                   |
|      * Total Plaintext Bytes: u64 BE                          |
|      * SHA-256 Hash of Full Plaintext: [u8; 32]               |
+---------------------------------------------------------------+
```

---

## 3. Cryptographic Parameters

### 3.1 Key Derivation
Master encryption key $K$ (32 bytes) is derived via:
$$K = \text{PBKDF2-HMAC-SHA256}(\text{Passphrase}, \text{Salt}, \text{Iterations}=100,000, 32)$$

### 3.2 Nonce Construction (12 Bytes)
Each chunk derives a strictly unique 12-byte nonce:
$$\text{Nonce}_i = \text{NoncePrefix}_{4\text{B}} \parallel \text{ChunkIndex}_{8\text{B BE}}$$
Because $\text{ChunkIndex}$ monotonically increases ($0, 1, 2, \dots$), nonces can never repeat within an archive, and $\text{NoncePrefix}$ ensures uniqueness across different archives using the same key.

### 3.3 Authenticated Additional Data (AAD) (29 Bytes)
Each chunk is authenticated against a 29-byte AAD header:
$$\text{AAD}_i = \text{ArchiveID}_{16\text{B}} \parallel \text{ChunkIndex}_{8\text{B BE}} \parallel \text{ChunkType}_{1\text{B}} \parallel \text{PlaintextLen}_{4\text{B BE}}$$

This guarantees:
- **No Chunk Swapping:** Chunk $i$ cannot be placed at position $j$.
- **No Archive Mixing:** Chunk from Archive $A$ cannot be inserted into Archive $B$.
- **No Truncation:** Missing chunks or altered lengths invalidate authentication.

---

## 4. Backwards Compatibility

When reading an encrypted backup file, `BackupManager::restore_encrypted_backup` inspects the first 8 bytes:
- If magic is `SPNEBAK\x02`: decrypts using streaming chunked pipeline.
- If magic is `SPNEBAK\x01`: transparently falls back to legacy single-pass decryptor.
- If rotated using `rotate_backup_key`: legacy v1 archives are upgraded automatically to SPNEBAK v2 streaming format.
