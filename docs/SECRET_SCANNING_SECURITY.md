# SECRET SCANNING & CREDENTIAL PROTECTION SPECIFICATION

**Date:** 2026-11-04  
**Batch Milestone:** Batch 14 — Full-System Integration, Security Hardening, Reliability, Recovery, and Performance  
**Auditor / Engineering Lead:** DeepMind Advanced Agentic Coding Pair Program  
**Workspace:** `d:\Vault\Spine`

---

## 1. SAFE FINGERPRINTING WITH KEYED HMAC-SHA256

Unkeyed SHA-256 hashes of high-entropy or low-entropy credentials remain vulnerable to rainbow table precomputation or offline dictionary cracking if the hash database leaks.

In Batch 14, Spine implements **keyed HMAC-SHA256 fingerprinting with key versioning** (`security/service.rs`):

```rust
pub fn compute_keyed_safe_fingerprint(raw_secret: &str, key_id: &str, key_bytes: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(key_bytes).unwrap();
    mac.update(raw_secret.as_bytes());
    let hmac_hex = hex::encode(mac.finalize().into_bytes());

    let prefix = &raw_secret[..4];
    let suffix = &raw_secret[raw_secret.len() - 4..];
    format!("{}...****...{} ({}:{})", prefix, suffix, key_id, &hmac_hex[..12])
}
```

### Guarantees
1. **Zero Plaintext Storage:** Plaintext secret tokens (`sp_pat_...`, `ghp_...`, `AKIA...`, private keys) are never persisted in the database, logged to tracing sinks, returned in API payloads, or exposed in search index snippets.
2. **Key Rotation Support:** Each fingerprint is prefixed with its key version (e.g. `hmac-sha256:v1`), enabling safe server-side key rotation without corrupting or invalidating historical findings.
3. **Redacted Previews:** Only the first 4 characters and last 4 characters are retained for developer identification (e.g., `sp_p...****...1a2b`), while the inner body is masked with `****`.

---

## 2. CARGO VULNERABILITY AUDITING & PROVENANCE

Spine's dependency auditor inspects `Cargo.lock` and `Cargo.toml` manifests and matches packages against security advisories.

### Provenance Tracking
The security service exposes explicit advisory feed provenance via `state.security.get_advisory_feed_provenance()`:
- **Source:** `"RustSec Advisory Database (crates.io)"`
- **Version:** `"2026.11.01-cargo"`
- **Status:**
  - `ACTIVE_INDEXED`: Feed loaded and active.
  - `NOT_CONFIGURED`: No advisories loaded.
  - `UNAVAILABLE`: Upstream network feed unreachable.
  - `STALE`: Feed has not been updated within configured freshness window.

Spine never fabricates synthetic advisory records and explicitly distinguishes between an absence of known vulnerabilities and an unconfigured/stale advisory database.
