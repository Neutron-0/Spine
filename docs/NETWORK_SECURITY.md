# Spine Network Security Architecture (Batch 05)

## 1. Overview
The Spine Version Control System is designed with zero-trust networking principles. All remote communication through the Spine Wire Protocol (`SPWP`) treats network links as hostile, eavesdrop-capable, and prone to packet tampering or connection reset.

Spine enforces four distinct layers of security:
1. **Transport Security (Confidentiality & Integrity):** TLS 1.3 encryption with modern AEAD cipher suites (`rustls 0.23` and `tokio-rustls 0.26` backed by `ring 0.17`) protecting against passive observation and MITM tampering. Unencrypted plaintext connections are restricted strictly to local loopback (`127.0.0.1` / `::1`) with active kernel peer inspection.
2. **Peer Identity & Asymmetric Authentication (Non-Repudiation):** Cryptographic challenge-response authentication using Ed25519 asymmetric keys with session-bound, domain-separated transcripts (`b"SPWP-AUTH-v1\0"`), CSPRNG nonces, and sliding-window replay caching (`BoundedReplayCache`).
3. **Repository Authorization (Access Control):** Fine-grained role-based access control (RBAC: `Reader`, `Contributor`, `Admin`) governing read, write, protected branches, and lease-based force-push operations.
4. **Push Object-Closure Validation (Transitive Graph Integrity):** Server independently traverses and validates complete transitive graph reachability (commits, trees, subtrees, blobs) before ref publication, defeating truncated or corrupt packs.

---

## 2. Threat Model & Mitigations

| Threat | Attack Vector | Spine Mitigation |
|---|---|---|
| **Eavesdropping** | Intercepting network frames on public or compromised LAN links | TLS 1.3 end-to-end encryption. Unencrypted connections are restricted strictly to loopback interfaces. |
| **Replay Attacks** | Capturing valid client authentication frames and resending them later | Server generates 32-byte cryptographic random nonces with millisecond timestamps. Nonces are tracked in `BoundedReplayCache` with a 120s sliding window. |
| **Identity Spoofing** | Sending unauthenticated usernames or user IDs | Client must prove private key possession via Ed25519 signature over deterministic transcript. User IDs are resolved from registered public keys. |
| **Repository Enumeration** | Probing unauthenticated servers to discover private branch names or commit hashes | Strict protocol sequencing: Reference advertisement (`REF_ADVERTISE`) is completely withheld until `AUTH_RESULT` evaluates to success. Private repositories return generic 404. |
| **Denial of Service (DoS)** | Sending malformed lengths or gigabyte-sized frame headers | Strict 16 MB maximum payload ceiling enforced before allocating memory buffers. Oversized frames abort connection immediately. Read timeouts set to 30s. Connection semaphore limits concurrency. |
| **Object Incompleteness / Ingestion Corruption** | Pushing commits with missing subtrees or blobs | Server performs recursive object closure validation before updating any reference. Incomplete pushes are rejected with `missing_object`. |
| **Race Conditions (Lost Updates)** | Two developers pushing concurrently to the same branch | Atomic Compare-and-Swap (`REF_UPDATE_REQ` requires `expected_old_oid`). `--force-with-lease` verifies expected remote tip. |

---

## 3. Cryptographic Handshake & Challenge-Response

```
Client                                                  Server
  |                                                       |
  | -------- [TLS 1.3 Handshake (rustls / ring)] -------> |
  | <------- [TLS 1.3 Session Established] -------------- |
  |                                                       |
  | -------- HELLO (client capabilities) ---------------> |
  | <------- HELLO_ACK (server capabilities) ------------ |
  |                                                       |
  | <------- AUTH_CHALLENGE (session_id, nonce, ts) ----- |
  |                                                       |
  | [Compute: BLAKE3("SPWP-AUTH-v1\0" || transcript)]     |
  | [Sign: Ed25519(Digest)]                               |
  |                                                       |
  | -------- AUTH_RESPONSE (pubkey, signature, scope) --> |
  |                                                       |
  |          [Verify signature against KeyRegistry]       |
  |          [Verify nonce against BoundedReplayCache]    |
  | <------- AUTH_RESULT (role: contributor, ok) -------- |
  |                                                       |
```

### Signature Construction
The client creates signature $\sigma$:
$$\text{Digest} = \text{BLAKE3}(b\text{"SPWP-AUTH-v1}\setminus0\text{"} \parallel \text{version} \parallel \text{server\_id} \parallel \text{session\_id} \parallel \text{nonce} \parallel \text{timestamp} \parallel \text{repo} \parallel \text{scope})$$
$$\sigma = \text{Ed25519\_Sign}(\text{SigningKey}, \text{Digest})$$

The server verifies:
1. Current wall time is within $[-120\text{s}, +60\text{s}]$ of `timestamp`.
2. `nonce` has not been seen in the active server `BoundedReplayCache`.
3. $\text{Ed25519\_Verify}(\text{VerifyingKey}, \text{Digest}, \sigma) = \text{true}$.
4. $\text{VerifyingKey}$ is registered and not revoked in `KeyRegistry`.

---

## 4. Authorization Matrix

| Permission | Public / Anonymous | Reader | Contributor | Admin |
|---|:---:|:---:|:---:|:---:|
| `REPO_OPEN` | Allowed (if public) | Allowed | Allowed | Allowed |
| `REF_ADVERTISE` | Allowed (if public) | Allowed | Allowed | Allowed |
| `FETCH_NEGOTIATE` | Allowed (if public) | Allowed | Allowed | Allowed |
| `PACK_STREAM_READ` | Allowed (if public) | Allowed | Allowed | Allowed |
| `PUSH_BEGIN` (Fast-forward) | Denied | Denied | Allowed | Allowed |
| `PUSH_WITH_LEASE` | Denied | Denied | Denied | Allowed |
| `REF_FORCE_UPDATE` | Denied | Denied | Denied | Allowed |
| `PROTECTED_BRANCH_UPDATE` | Denied | Denied | Denied | Allowed |
