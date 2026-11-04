# Spine Forge — Git LFS Authorization & Scoped Tokens

## 1. Authentication Mechanisms

Git LFS operations support two complementary authentication paths:

1. **HTTP Basic Authentication (Client to Batch API):**
   - Username: Forge username.
   - Password: Personal Access Token (`sp_pat_...`).
   - Ordinary account passwords may be supported in dev/test, but PATs with `repo` scope are strictly recommended.
   - Evaluated using constant-time hash checks against `PersonalAccessToken.token_hash`.

2. **Short-Lived Scoped Transfer Tokens (Batch API to Transfer Actions):**
   - Issued in `actions.<op>.header.Authorization = "Bearer <token>"`.
   - Forwarded automatically by Git LFS clients during `PUT`, `GET`, and `POST verify`.

---

## 2. Scoped Token Architecture

To avoid transmitting long-lived credentials to secondary transfer hosts or embedding permanent capabilities in URLs, transfer actions use scoped tokens:

```
<payload_hex>.<hmac_sha256_signature_hex>
```

### Token Payload Fields:
- `repo_id`: UUID of the authorized repository.
- `oid`: Exact SHA-256 hex OID of the object.
- `op`: Permitted action (`"upload"`, `"download"`, or `"verify"`).
- `max_size`: Maximum allowed payload size.
- `exp`: Unix expiration timestamp (TTL 3600 seconds).

### Security Fencing Guarantees:
- **No Cross-Repository Reuse:** A token generated for `repo-A` cannot be used to upload or download objects from `repo-B`.
- **No Operation Substitution:** An `upload` token cannot download objects; a `download` token cannot upload.
- **No OID Substitution:** The token signature binds to the exact target OID.
- **Constant-Time Verification:** Signatures are evaluated via `subtle::ConstantTimeEq` to prevent timing attacks.
- **Automatic Expiration:** Tokens expire after 1 hour (3600s).

---

## 3. Repository Visibility & Isolation

- **Public Repositories:**
  - Anonymous read for Batch download actions and `GET` transfers.
  - Authenticated write access required for Batch upload actions and `PUT` transfers.
- **Private Repositories:**
  - Unauthenticated requests receive HTTP `401 Unauthorized` with `WWW-Authenticate: Basic realm="Spine Forge"`.
  - Batch API and transfer endpoints enforce collaborator / organization membership before revealing object presence or actions.
  - Zero cross-tenant leakage: existence of an object in another private repository is never disclosed.
