# Spine Network Security Architecture

## 1. Overview
The Spine Version Control System is designed with zero-trust networking principles. All remote communication through the Spine Wire Protocol (`SPWP`) treats network links as hostile, eavesdrop-capable, and prone to packet tampering or connection reset.

Spine enforces three distinct layers of security:
1. **Transport Security (Confidentiality & Integrity):** TLS 1.3 encryption with modern cipher suites (ChaCha20-Poly1305, AES-256-GCM) protecting against passive observation and MITM tampering.
2. **Peer Identity & Authentication (Non-Repudiation):** Cryptographic challenge-response authentication using Ed25519 asymmetric keys with session-bound monotonic nonces.
3. **Repository Authorization (Access Control):** Fine-grained role-based access control (RBAC) governing read, write, force-push, and administrative operations.

---

## 2. Threat Model

| Threat | Attack Vector | Spine Mitigation |
|---|---|---|
| **Eavesdropping** | Intercepting network frames on public or compromised LAN links | TLS 1.3 end-to-end encryption. Unencrypted connections are restricted to local loopback (`127.0.0.1` / `::1`). |
| **Replay Attacks** | Capturing valid client authentication frames and resending them later | Server generates high-entropy 32-byte cryptographic random nonces with millisecond timestamps. Nonces expire after 60 seconds and cannot be reused. |
| **Metadata Leakage** | Probing unauthenticated servers to discover private branch names or commit hashes | Strict protocol sequencing: Reference advertisement (`REF_ADVERTISE`) is completely withheld until `AUTH_RESULT` evaluates to success. |
| **Denial of Service (DoS)** | Sending malformed lengths or gigabyte-sized frame headers | Strict 16 MB maximum payload ceiling enforced before allocating memory buffers. Oversized frames abort connection immediately. |
| **Object Spoofing / Tampering** | Injecting modified tree or commit payloads during pack streaming | Every object is content-addressed with BLAKE3 or SHA-256. The server recalculates and validates every object's hash during ingestion. Mismatched objects abort the transaction. |
| **Race Conditions (Lost Updates)** | Two developers pushing concurrently to the same branch | Atomic Compare-and-Swap (`REF_UPDATE_REQ` requires `expected_old_oid`). The second push fails immediately with `stale_ref`. |

---

## 3. Cryptographic Handshake & Challenge-Response
The authentication exchange executes as follows:

```
Client                                                  Server
  |                                                       |
  | -------- HELLO (client capabilities) ---------------> |
  | <------- HELLO_ACK (server capabilities) ------------ |
  |                                                       |
  | <------- AUTH_CHALLENGE (32-byte nonce, timestamp) -- |
  |                                                       |
  | [Sign: Ed25519(nonce || timestamp || user_id)]        |
  |                                                       |
  | -------- AUTH_RESPONSE (pubkey, signature, user) ---> |
  |                                                       |
  |          [Verify signature against authorized keys]   |
  | <------- AUTH_RESULT (role: contributor, ok) -------- |
  |                                                       |
```

### Signature Construction
The client creates signature $\sigma$:
$$\sigma = \text{Ed25519\_Sign}(K_{\text{private}}, \text{nonce} \mathbin{\Vert} \text{timestamp} \mathbin{\Vert} \text{user\_id})$$

The server verifies:
1. Current wall time is within $\pm 60$ seconds of `timestamp`.
2. `nonce` has not been seen in the active server replay window.
3. $\text{Ed25519\_Verify}(K_{\text{public}}, \text{payload}, \sigma) = \text{true}$.
4. $K_{\text{public}}$ is registered and authorized for the requested repository.

---

## 4. Authorization Matrix
Repositories define role privileges:

| Permission | Public / Anonymous | Reader | Contributor | Admin |
|---|---|---|---|---|
| `REPO_OPEN` | Allowed (if public) | Allowed | Allowed | Allowed |
| `REF_ADVERTISE` | Allowed (if public) | Allowed | Allowed | Allowed |
| `FETCH_NEGOTIATE` | Allowed (if public) | Allowed | Allowed | Allowed |
| `PACK_STREAM_READ` | Allowed (if public) | Allowed | Allowed | Allowed |
| `PUSH_BEGIN` (Fast-forward) | Denied | Denied | Allowed | Allowed |
| `REF_FORCE_UPDATE` | Denied | Denied | Denied | Allowed |
| `REPO_DELETE` / `CONFIG` | Denied | Denied | Denied | Allowed |

Any unauthorized frame received on a connection generates a structured `0x0060` `ERROR` frame (`403 Forbidden`) and halts operation.
