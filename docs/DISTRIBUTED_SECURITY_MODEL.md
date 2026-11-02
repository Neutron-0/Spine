# Spine Distributed Security Model

## 1. Threat Model & Security Boundaries

The Spine distributed cluster is designed to operate securely across untrusted local-area or wide-area networks. It anticipates the following threat vectors:
- **Rogue / Unauthorized Node Injection:** An attacker attempts to join a malicious node to intercept repository data or serve counterfeit objects.
- **Split-Brain Write Injection:** A deposed or partitioned primary attempts to write stale data back into the repository.
- **Replay & Tampering Attacks:** An attacker replays captured heartbeat, replication, or transaction RPC messages.
- **Data Corruption at Rest:** Silent bit rot, failing media, or unauthorized filesystem alterations.

---

## 2. Cryptographic Node Enrollment

Storage nodes cannot participate in the cluster without completing an authenticated challenge-response enrollment handshake (`spine_cluster::enrollment`):

```
       Coordinator                                    Storage Node
            |                                               |
            |   [Pre-Shared Out-of-Band Enrollment Token]   |
            | <-------------------------------------------> |
            |                                               |
            |                                               | Generate Ed25519 Keypair
            |                                               | Construct EnrollmentRequest
            | <--------- EnrollmentRequest ---------------- | { node_id, public_key,
            |            (Signed with Node Key)             |   token, timestamp, signature }
            |                                               |
    1. Validate Token Expiration & Nonce                    |
    2. Verify Ed25519 Signature over Token                  |
    3. Register Node in Authoritative Store                 |
            |                                               |
            | --------- EnrollmentSuccess ----------------> |
            |           (Signed with Coordinator Key)       |
            v                                               v
```

### 2.1 Cryptographic Primitives
- **Ed25519 (`ed25519-dalek`):** Asymmetric signatures for node identity and challenge-response authentication.
- **BLAKE3:** Fast, cryptographic 256-bit hash tree algorithm used for:
  - Repository pack checksums (`pack_checksum`).
  - Node identity fingerprints.
  - Message integrity validation.
- **CSPRNG (`rand::rngs::OsRng`):** Secure token generation for enrollment authorization.

---

## 3. Fencing Tokens & Replay Defense

Every cluster mutating operation carries an `epoch` sequence number and monotonic timestamp:
1. **Epoch Enforcement:** A storage node or coordinator rejects any request whose epoch is strictly less than the repository's current recorded epoch ($E_{\text{req}} < E_{\text{current}}$).
2. **Heartbeat Replay Prevention:** Storage node heartbeats must contain monotonically increasing timestamps. Heartbeats with timestamps older than the last acknowledged ping or skewed by more than 300 seconds are discarded.
3. **Immutability of Pack Files:** Once committed to `<objects>/pack/`, `.spk` packfiles are immutable. Any modification alters the BLAKE3 digest and is flagged by integrity scans as `CorruptedPack`.
