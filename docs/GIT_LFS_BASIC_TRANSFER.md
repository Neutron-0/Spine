# Spine Forge — Git LFS Basic Transfer Protocol

## 1. Transfer Adapter Implementation

The official Git LFS Basic Transfer Adapter uses HTTP `PUT` for upload, HTTP `POST` for verification, and HTTP `GET` for download.

---

## 2. Upload Endpoint

- **Method & Route:**
  `PUT /:owner/:repo_git/info/lfs/objects/:oid`
- **Request Headers:**
  - `Authorization: Bearer <scoped_token>` (or `Basic <base64>`)
  - `Content-Type: application/octet-stream`
  - `Content-Length: <size>`
- **Processing Pipeline:**
  1. Authenticates scoped token or Basic credentials; verifies repository write permission.
  2. Bounded body stream: verifies received byte length matches declared size.
  3. Incremental hash calculation: computes SHA-256 and BLAKE3 running digests.
  4. Integrity verification: asserts computed SHA-256 equals path `:oid`. If mismatch occurs, returns `400 Bad Request` and discards all staging data.
  5. Chunking & persistence: stream-chunks into 4 MiB blocks via `spine-artifact::StreamChunker`. Chunks are written to `LocalArtifactStore` (`chunks/xx/<hash>`).
  6. Manifest creation: constructs `ArtifactManifest` recording chunk offsets, hashes, and total size.
  7. Database publication: stores `StoredArtifactManifest` in `state.db`.
  8. Quota adjustment: updates owner logical and physical quota usage atomically.
  9. Returns `200 OK`.

---

## 3. Verification Endpoint

- **Method & Route:**
  `POST /:owner/:repo_git/info/lfs/verify`
- **Request Headers:**
  - `Authorization: Bearer <scoped_token>` (or `Basic <base64>`)
  - `Content-Type: application/vnd.git-lfs+json`
- **Request Body:**
  ```json
  {
    "oid": "e3b0c442...",
    "size": 10485760
  }
  ```
- **Validation:**
  - Confirms object manifest exists in repository storage.
  - Verifies total recorded size matches declared `size`.
  - Idempotent: safe for multiple retries by client.
  - Returns `200 OK` with `{}`.

---

## 4. Download Endpoint

- **Method & Route:**
  `GET /:owner/:repo_git/info/lfs/objects/:oid`
- **Request Headers:**
  - `Authorization: Bearer <scoped_token>` (or `Basic <base64>`)
- **Response Headers:**
  - `Content-Type: application/octet-stream`
  - `Content-Length: <size>`
- **Response Payload:**
  - Raw binary stream reconstructed chunk-by-chunk from `spine-artifact` content-addressed storage.
  - Never returns JSON envelopes, chunk metadata, or LFS pointer text.
