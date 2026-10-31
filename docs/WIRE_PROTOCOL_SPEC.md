# Spine Wire Protocol Specification (SPWP v1)

## 1. Scope and Abstract
This document defines the formal binary wire protocol for the Spine Version Control System (`SPWP` Version 1). The protocol governs client-server communication across local loopback, intranet networks, and public Internet connections. It specifies byte-level framing, handshake sequencing, cryptographic peer authentication, object graph negotiation (`WANT`/`HAVE`), streaming packfile transmission, atomic compare-and-swap reference updates, and error signaling.

---

## 2. Transport and URL Notation
Spine remote repositories are addressed using standard Uniform Resource Identifiers:

```text
spine://[<user>@]<hostname>[:<port>]/<repository-relative-path>
```

- **Scheme:** `spine` (or `spinetls` for explicitly pinned TLS).
- **Default Port:** `4433` for production TLS 1.3 connections; `4434` for plaintext local development/daemon testing.
- **Path Semantics:** Identifies the repository path relative to the daemon's root storage directory.
- **Underlying Stream:** Reliable, bidirectional ordered octet stream (TCP, TLS 1.3, or encrypted loopback sockets).

---

## 3. Frame Layout and Binary Encoding
All network messages in `SPWP` are transmitted as self-delimiting binary frames. Numbers are encoded in **Little-Endian** byte order.

```text
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                       Magic: "SPWP"                           |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|       Protocol Version        |          Frame Type           |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|         Frame Flags           |    (Reserved - must be 0)     |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                        Payload Length                         |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                    Payload BLAKE3 Checksum                    |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
|                                                               |
+                     Payload Data (L bytes)                    +
|                                                               |
+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
```

### 3.1 Field Definitions
| Offset | Field | Type | Description |
|---|---|---|---|
| `0..4` | Magic | 4 bytes | Literal ASCII `b"SPWP"` (`0x53, 0x50, 0x57, 0x50`). Any frame without this magic is immediately rejected. |
| `4..6` | Protocol Version | `u16` LE | Fixed to `1` (`0x0001`) for SPWP v1. |
| `6..8` | Frame Type | `u16` LE | Registry value identifying message semantics (see Section 4). |
| `8..10` | Frame Flags | `u16` LE | Bitfield: `0x0001` = End of Stream, `0x0002` = Compressed Payload, `0x0004` = Heartbeat probe. |
| `10..12` | Reserved | `u16` LE | Reserved for future expansion; must be sent as `0x0000`. |
| `12..16` | Payload Length | `u32` LE | Length $L$ of payload in bytes ($0 \le L \le 16,777,216$). Frames exceeding 16 MB trigger a `FrameTooLarge` error and connection termination. |
| `16..20` | Payload Checksum | 4 bytes | First 4 bytes of BLAKE3 hash of `Payload Data`. If $L = 0$, checksum is `0x00000000`. |
| `20..20+L` | Payload Data | $L$ bytes | Message-specific encoded binary or structured payload. |

Total fixed header overhead: **20 bytes**.

---

## 4. Frame Registry and Message Payloads

### 4.1 Handshake Messages
- **`0x0001` — `HELLO` (Client $\to$ Server)**
  - Payload: JSON object:
    ```json
    {
      "client_version": "0.1.0",
      "capabilities": ["pack_zstd", "ed25519_auth", "cas_push"],
      "agent": "spine-cli/0.1.0 (windows-x86_64)"
    }
    ```
- **`0x0002` — `HELLO_ACK` (Server $\to$ Client)**
  - Payload: JSON object:
    ```json
    {
      "server_version": "0.1.0",
      "selected_version": 1,
      "capabilities": ["pack_zstd", "ed25519_auth", "cas_push"],
      "server_id": "spine-daemon-node-01"
    }
    ```

### 4.2 Authentication Messages
- **`0x0010` — `AUTH_CHALLENGE` (Server $\to$ Client)**
  - Payload: 32 bytes random nonce + 8 bytes timestamp (`u64` LE Unix milliseconds) = 40 bytes.
- **`0x0011` — `AUTH_RESPONSE` (Client $\to$ Server)**
  - Payload:
    - `user_id_len`: `u16` LE.
    - `user_id`: UTF-8 string bytes.
    - `public_key`: 32 bytes (Ed25519 public key).
    - `signature`: 64 bytes (Ed25519 signature over `nonce || timestamp || user_id`).
- **`0x0012` — `AUTH_RESULT` (Server $\to$ Client)**
  - Payload: JSON object:
    ```json
    {
      "authenticated": true,
      "user": "alice",
      "role": "contributor",
      "message": "Authentication successful"
    }
    ```

### 4.3 Repository Selection & Reference Advertisement
- **`0x0020` — `REPO_OPEN` (Client $\to$ Server)**
  - Payload: JSON: `{"path": "org/repo"}`.
- **`0x0021` — `REPO_INFO` (Server $\to$ Client)**
  - Payload: JSON:
    ```json
    {
      "algorithm": "blake3",
      "default_branch": "main",
      "storage_format": "v2",
      "head_oid": "4f1a..."
    }
    ```
- **`0x0030` — `REF_ADVERTISE` (Server $\to$ Client)**
  - Advertises remote branches and tags.
  - Payload: JSON map of reference names to 64-char hex Object IDs:
    ```json
    {
      "refs": {
        "refs/heads/main": "4f1a2380598a...",
        "refs/heads/feature-x": "9e2c4510...",
        "refs/tags/v1.0.0": "2a81cf00..."
      }
    }
    ```

### 4.4 Object Graph Negotiation (Fetch / Clone)
- **`0x0040` — `FETCH_NEGOTIATE` (Client $\to$ Server)**
  - Payload:
    ```json
    {
      "want": ["4f1a2380..."],
      "have": ["9e2c4510..."]
    }
    ```
- **`0x0041` — `FETCH_PLAN` (Server $\to$ Client)**
  - Payload:
    ```json
    {
      "objects_to_send": 42,
      "estimated_bytes": 1048576,
      "already_up_to_date": false
    }
    ```

### 4.5 Streaming Pack Transfer
- **`0x0050` — `PUSH_BEGIN` (Client $\to$ Server)**
  - Initiates push pack transmission. Payload: JSON with client metadata and expected ref updates.
- **`0x0051` — `PACK_DATA` (Bidirectional: Server $\to$ Client during Fetch, Client $\to$ Server during Push)**
  - Raw binary chunk of `.spk` packfile stream.
  - Stream chunks are written incrementally to `.incoming-<uuid>.spk`.
- **`0x0052` — `PACK_PROGRESS` (Sender $\to$ Receiver)**
  - Out-of-band progress reporting. Payload: JSON: `{"phase": "compressing", "current": 25, "total": 100, "message": "Delta calculation"}`.
- **`0x0053` — `PACK_END` (Sender $\to$ Receiver)**
  - Indicates pack transmission completion.
  - Payload: 32 bytes (expected cumulative BLAKE3 checksum of the complete `.spk` file).

### 4.6 Compare-and-Swap Reference Update
- **`0x0031` — `REF_UPDATE_REQ` (Client $\to$ Server)**
  - Payload:
    ```json
    {
      "updates": [
        {
          "ref_name": "refs/heads/main",
          "expected_old_oid": "4f1a2380...",
          "new_oid": "89ab0123...",
          "force": false
        }
      ]
    }
    ```
- **`0x0032` — `REF_UPDATE_RES` (Server $\to$ Client)**
  - Payload:
    ```json
    {
      "success": true,
      "results": [
        {
          "ref_name": "refs/heads/main",
          "status": "ok",
          "current_oid": "89ab0123..."
        }
      ]
    }
    ```
  - Status codes: `ok`, `stale_ref`, `non_fast_forward`, `permission_denied`, `object_missing`.

### 4.7 Diagnostics and Keepalive
- **`0x0060` — `ERROR` (Bidirectional)**
  - Payload: JSON: `{"code": 404, "category": "repository_not_found", "message": "Repository does not exist"}`.
- **`0x0070` — `PING` (Bidirectional)**
  - Empty or 4-byte echo token.
- **`0x0071` — `PONG` (Bidirectional)**
  - Echoes the token received in `PING`.

---

## 5. Security & Invariant Guarantees
1. **Zero Metadata Leakage:** No repository branch names or commit hashes are advertised before successful authentication (frame `0x0012` `AUTH_RESULT`).
2. **Atomic Ingestion:** Incoming packs are isolated in `.incoming-<uuid>.spk` until checksums match and object graph traversal proves completeness. Partial or interrupted streams are unlinked immediately without polluting the repository catalog.
3. **Compare-and-Swap Race Prevention:** Any push proposal that fails to match the server's current advertised reference OID is atomically rejected with `stale_ref`, preventing accidental history overwrites.
