# SPNEBAK v2 Format Specification Reference

Refer to [BACKUP_ENCRYPTION_V2.md](file:///d:/Vault/Spine/docs/BACKUP_ENCRYPTION_V2.md) for the complete cryptographic architecture and binary structure of the `SPNEBAK\x02` backup archive format.

### Summary of Magic Bytes & Constants
- **V1 Magic (Legacy):** `b"SPNEBAK\x01"` (Header length: 42 bytes)
- **V2 Magic (Streaming Chunked):** `b"SPNEBAK\x02"` (Header length: 54 bytes)
- **Chunk Types:**
  - `0x01`: `CHUNK_TYPE_DATA` (contains encrypted tar payload)
  - `0x02`: `CHUNK_TYPE_FINAL` (contains authenticated stream summary manifest)
- **Default Chunk Size:** 1,048,576 bytes (1 MiB)
- **Tag Length:** 16 bytes (AES-GCM-256 standard tag)
- **Memory Footprint:** $\le 2 \text{ MiB}$ constant heap usage regardless of input size.
