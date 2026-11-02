# Spine Cryptographic Peer Identity, Asymmetric Authentication, and RBAC

## 1. Executive Summary

In Batch 05, Spine transitions from symmetric BLAKE3 keyed-hash MACs to **cryptographic asymmetric identity** using the **Ed25519** digital signature algorithm (`RFC 8032`).

Users, services, and daemons prove their identity through digital signatures over deterministic, domain-separated challenge transcripts. Client-supplied usernames or user IDs are never trusted without cryptographic signature verification against a server-side `KeyRegistry`.

---

## 2. Peer Identity Data Structures

### 2.1 The Ed25519 Keypair
Identities are managed via `SpineIdentity`:
- **Private Key**: 32-byte secret seed generating an Ed25519 `SigningKey` (`ed25519-dalek 2.2` with `zeroize` on drop).
- **Public Key**: 32-byte compressed Edwards point representing the `VerifyingKey`.

### 2.2 Public Key Fingerprint Specification
Public key fingerprints identify keys in logs, audit trails, and CLI output:
$$\text{Digest} = \text{BLAKE3}(b\text{"SPWP-KEY-v1}\setminus0\text{"} \parallel \text{PublicKeyBytes}_{32})$$
$$\text{Fingerprint} = \text{"spkey1:"} \parallel \text{hex}(\text{Digest}[0..16])$$

Example fingerprint:
```
spkey1:4f8a29e160c883e4bc3911029e84d720
```

---

## 3. Challenge-Response Protocol Specification

### 3.1 Challenge Generation
Upon establishing a version-negotiated connection, the server generates:
- `nonce`: 32-byte cryptographically secure random value generated via `rand::rngs::OsRng`.
- `timestamp_ms`: Current server UNIX epoch time in milliseconds.
- `session_id`: 32-bit ephemeral connection identifier.
- `server_id`: UTF-8 server node identifier.

### 3.2 The Domain-Separated Authentication Transcript
The client constructs a deterministic binary byte stream and hashes it with BLAKE3:

$$\text{Domain Separator} = b\text{"SPWP-AUTH-v1}\setminus0\text{"} \quad (13 \text{ bytes})$$

The transcript encompasses:
```
[13 bytes] Domain Separator: b"SPWP-AUTH-v1\x00"
[ 2 bytes] Protocol Version: u16 (little-endian)
[ 4 bytes] Server ID Length: u32 (little-endian)
[ N bytes] Server ID Bytes:  UTF-8
[ 4 bytes] Session ID:       u32 (little-endian)
[32 bytes] Nonce:            [u8; 32]
[ 8 bytes] Timestamp MS:     u64 (little-endian)
[ 4 bytes] Repo Path Length: u32 (little-endian)
[ M bytes] Repo Path Bytes:  UTF-8
[ 4 bytes] Scope Length:     u32 (little-endian)
[ K bytes] Scope Bytes:      UTF-8
```

The 32-byte digest is signed using Ed25519:
$$\sigma = \text{Ed25519Sign}(\text{SigningKey}, \text{Digest})$$

### 3.3 Replay Protection Cache (`BoundedReplayCache`)
The server protects against replay attacks using a thread-safe sliding-window cache:
1. **Timestamp Window Check**:
   $$\Delta t = |t_{\text{server}} - t_{\text{challenge}}|$$
   - Reject if $t_{\text{challenge}} > t_{\text{server}} + 60{,}000\text{ ms}$ (clock skew forward).
   - Reject if $t_{\text{server}} - t_{\text{challenge}} > 120{,}000\text{ ms}$ (stale challenge).
2. **Nonce Deduplication**:
   If `nonce` exists in `BoundedReplayCache`, reject immediately with `AuthError::ReplayDetected`.
3. **Bounded Memory Eviction**:
   Maximum capacity is bounded to 20,000 entries. Expired nonces past 120s are pruned during capacity checks, completely defeating memory exhaustion DoS.

---

## 4. Role-Based Access Control (RBAC) Matrix

Every verified identity maps to an authorized principal and role in the server's `KeyRegistry`:

| Role | Clone & Fetch | Push Fast-Forward | Push with Lease | Force Push | Modify Protected Branches |
|---|:---:|:---:|:---:|:---:|:---:|
| `Reader` | Yes | No | No | No | No |
| `Contributor` | Yes | Yes (Non-protected) | No | No | No |
| `Admin` | Yes | Yes | Yes | Yes | Yes |

### Key Revocation and Expiration
Public keys support:
- `is_revoked: bool`: Instant revocation flag rejecting authentication with `AuthError::KeyRevoked`.
- `expires_at: Option<u64>`: Temporal expiration rejecting credentials past their lifetime with `AuthError::KeyExpired`.
