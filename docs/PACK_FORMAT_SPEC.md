# Spine Packfile Binary Specification (V1)
**Document ID:** SPEC-VCS-PACK-0001  
**Format Version:** 1.0  
**Status:** Canonical Engineering Specification  
**Architecture:** 64-bit Little-Endian, Bounded Allocation  

---

## 1. Overview
The Spine Version Control System employs an immutable, dual-file architecture for packaged object storage:
1. **`.spk` (Spine Packfile):** Sequential stream of content-addressed, cryptographic object records with header verification and cumulative trailer hashing.
2. **`.spidx` (Spine Pack Index):** Companion binary search index utilizing a 256-bucket fanout table and sorted 32-byte object identifiers to achieve $O(1)$ amortized and $O(\log_2 (N/256))$ worst-case random object lookups.

All integers in `.spk` and `.spidx` are encoded in **standard Little-Endian byte order**.

---

## 2. Packfile (`.spk`) Byte-Level Specification

A `.spk` file consists of three contiguous regions:
`[ Header: 16 bytes ]` $\to$ `[ Sequence of N Records ]` $\to$ `[ Trailer: 32 bytes ]`

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                       Magic: "SPPK"                           |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|       Format Version (u16)    |     Hash Algorithm (u16)      |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                      Object Count N (u32)                     |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                     Header Checksum (u32)                     |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                                                               |
|                   Record 1 ... Record N                       |
|                                                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                       Magic: "SPTR"                           |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                                                               |
|                  Cumulative BLAKE3 (28 bytes)                 |
|                                                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

### 2.1 Pack Header (16 Bytes)
| Offset | Type | Field | Description |
|---|---|---|---|
| 0x00 | `[u8; 4]` | `magic` | Exact ASCII bytes `b"SPPK"` (`0x53, 0x50, 0x50, 0x4B`). |
| 0x04 | `u16` | `version` | Format version. Must be `0x0001`. |
| 0x06 | `u16` | `hash_algo` | Content hash algorithm: `0x0001` = BLAKE3, `0x0002` = SHA-256. |
| 0x08 | `u32` | `object_count` | Number of object records $N$ in this packfile ($0 \le N \le 2^{32}-1$). |
| 0x0C | `u32` | `header_checksum` | BLAKE3 hash of bytes `0x00..0x0C`, truncated to 4 bytes Little-Endian. |

### 2.2 Pack Object Record Framing
Each of the $N$ records has the following layout:
```
+---------------------------------------------------------------+
| Type (1B) | Comp (1B) | Reserved (2B)                         |
+---------------------------------------------------------------+
| Uncompressed Length L_orig (u64 LE, 8B)                       |
+---------------------------------------------------------------+
| Stored Length L_stored (u64 LE, 8B)                           |
+---------------------------------------------------------------+
| Object ID (32B)                                               |
+---------------------------------------------------------------+
| Payload Checksum (16B)                                        |
+---------------------------------------------------------------+
| Stored Payload (L_stored bytes) ...                           |
+---------------------------------------------------------------+
```

| Field | Offset | Type | Description |
|---|---|---|---|
| `object_type` | 0x00 | `u8` | `0x01` = Blob, `0x02` = Tree, `0x03` = Commit, `0x04` = Tag. |
| `compression` | 0x01 | `u8` | `0x00` = Raw (uncompressed), `0x01` = Zstandard (RFC 8878). |
| `reserved` | 0x02 | `[u8; 2]` | Must be initialized to `[0x00, 0x00]`. Reserved for delta encoding. |
| `orig_len` | 0x04 | `u64` | Original uncompressed byte count $L_{orig}$. Enforces safety limit $L_{orig} \le 4\text{ GB}$. |
| `stored_len` | 0x0C | `u64` | Byte count of payload as stored in packfile $L_{stored}$. |
| `object_id` | 0x14 | `[u8; 32]` | Canonical domain-separated `ObjectId` of the uncompressed object. |
| `payload_checksum`| 0x34 | `[u8; 16]` | BLAKE3 hash of the following $L_{stored}$ payload bytes, truncated to 16 bytes. |
| `payload` | 0x44 | `[u8; L_stored]` | Payload data (either Zstd compressed stream or raw uncompressed bytes). |

### 2.3 Pack Trailer (32 Bytes)
| Offset | Type | Field | Description |
|---|---|---|---|
| 0x00 | `[u8; 4]` | `magic` | Exact ASCII bytes `b"SPTR"` (`0x53, 0x50, 0x54, 0x52`). |
| 0x04 | `[u8; 28]` | `cumulative_hash` | BLAKE3 hash computed over all preceding bytes of the packfile (from byte 0 to the end of the $N$-th record payload), truncated to 28 bytes. |

---

## 3. Pack Index (`.spidx`) Byte-Level Specification

A companion `.spidx` index provides direct $O(1)$ fanout and binary search access:

```
+---------------------------------------------------------------+
| Header: 16 bytes ("SPIX", version, hash_algo, count, crc)     |
+---------------------------------------------------------------+
| 256-Entry Fanout Table: 1,024 bytes (256 x u32 LE)            |
+---------------------------------------------------------------+
| Sorted Object IDs: N x 32 bytes (sorted ascending)            |
+---------------------------------------------------------------+
| Record Offsets & Sizes: N x 16 bytes                          |
|   - Offset in .spk (u64 LE, 8B)                               |
|   - Stored Length (u32 LE, 4B)                                |
|   - Original Length (u32 LE, 4B)                              |
+---------------------------------------------------------------+
| Trailer: 32 bytes ("SIXT" + 28B BLAKE3)                       |
+---------------------------------------------------------------+
```

### 3.1 Index Header (16 Bytes)
| Offset | Type | Field | Description |
|---|---|---|---|
| 0x00 | `[u8; 4]` | `magic` | Exact ASCII bytes `b"SPIX"` (`0x53, 0x50, 0x49, 0x58`). |
| 0x04 | `u16` | `version` | Index format version: `0x0001`. |
| 0x06 | `u16` | `hash_algo` | Hash algorithm identifier: `0x0001` = BLAKE3, `0x0002` = SHA-256. |
| 0x08 | `u32` | `object_count` | Number of indexed objects $N$. Must match `.spk` object count. |
| 0x0C | `u32` | `header_checksum` | BLAKE3 hash of bytes `0x00..0x0C` truncated to 4 bytes Little-Endian. |

### 3.2 256-Bucket Fanout Table (1,024 Bytes)
The fanout table contains 256 sequential `u32` Little-Endian integers ($F[0], F[1], \dots, F[255]$).
- $F[i]$ is the cumulative count of objects whose 32-byte `ObjectId` begins with a first byte $\le i$.
- For any prefix byte $b \in [0, 255]$:
  - Low index in sorted array: $L = \text{if } b == 0 \text{ then } 0 \text{ else } F[b-1]$
  - High index in sorted array: $H = F[b]$
  - Search range is $[L, H)$, containing exactly $F[b] - L$ elements.
- Invariant: $0 \le F[0] \le F[1] \le \dots \le F[255] = N$.

### 3.3 Sorted Object ID Array ($N \times 32$ Bytes)
Contiguous array of $N$ 32-byte `ObjectId`s sorted in strict lexicographical ascending order.
Allows standard binary search across slice $[L, H)$ in $\lceil \log_2 (H - L) \rceil \le 8$ iterations.

### 3.4 Offset and Size Table ($N \times 16$ Bytes)
A parallel array of $N$ 16-byte records mapped 1:1 with the sorted Object ID array:
```
+---------------------------------------------------------------+
| Pack Offset in .spk (u64 LE, 8B)                              |
+-------------------------------+-------------------------------+
| Stored Length (u32 LE, 4B)    | Original Length (u32 LE, 4B)  |
+-------------------------------+-------------------------------+
```

### 3.5 Index Trailer (32 Bytes)
| Offset | Type | Field | Description |
|---|---|---|---|
| 0x00 | `[u8; 4]` | `magic` | Exact ASCII bytes `b"SIXT"` (`0x53, 0x49, 0x58, 0x54`). |
| 0x04 | `[u8; 28]` | `cumulative_hash` | BLAKE3 hash computed over all preceding bytes of the index file (from byte 0 through the end of the offset table), truncated to 28 bytes. |

---

## 4. Multi-Pack Catalog Specification (`catalog.json`)

To orchestrate multiple packfiles without global lock contention:
Location: `.spine/objects/pack/catalog.json`

```json
{
  "format_version": 1,
  "generation": 4,
  "active_packs": [
    {
      "id": "pack-018eb2c4-857a-7230-9b62-12480bf13901",
      "object_count": 84210,
      "pack_size_bytes": 45182931,
      "index_size_bytes": 4042896,
      "pack_checksum": "a82f...39b1",
      "created_at_nanos": 1730000000000000000
    }
  ],
  "total_objects": 84210,
  "updated_at_nanos": 1730000000000000000
}
```

### 4.1 Publication Protocol
1. Packfile and companion index are written to temporary files: `.tmp-<pack_id>.spk` and `.tmp-<pack_id>.spidx`.
2. Both files are synced to disk via `fsync` (`sync_all()`).
3. Both files are atomically renamed into place.
4. An updated `catalog.json` with an incremented `generation` counter is prepared, synced, and atomically renamed.
5. If the system crashes at any point prior to step 4, the unreferenced temporary files or orphan packfiles are cleaned up safely during `spine fsck` or `spine gc`.
