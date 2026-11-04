# RELEASE ASSETS DISTRIBUTION

## 1. Architecture
Release assets are binary files attached to a release (e.g. native installers, compiled executables, source archives, SDK tarballs).

Assets are stored in the unified content-addressed artifact store (`spine-artifact`), benefiting from:
- Streaming chunking into 4 MiB blocks.
- Whole-object and per-chunk BLAKE3 checksums.
- Automatic deduplication across builds and releases.
- Atomic download streaming with bounded memory consumption.

---

## 2. Asset Schema

```json
{
  "id": "ast_01JBMY2K...",
  "release_id": "rel_01JBMX9Z...",
  "name": "spine-x86_64-pc-windows-msvc.zip",
  "size_bytes": 45182904,
  "content_type": "application/zip",
  "downloads_count": 142,
  "manifest_id": "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
  "created_at_nanos": 1793798400000000000
}
```

---

## 3. Upload & Download Flow

### Uploading an Asset
```bash
POST /api/v1/repos/:owner/:name/releases/:release_id/assets?name=spine.zip
Headers:
  Content-Type: application/zip
  Authorization: Bearer <PAT>
Body:
  <Raw Binary Stream>
```
The server streams the body directly through `StreamChunker`, stores chunk files, generates an `ArtifactManifest`, updates the repository quota, and creates the `release_assets` record.

### Downloading an Asset
```bash
GET /api/v1/repos/:owner/:name/releases/:release_id/assets/:asset_id/download
```
- Increments `downloads_count` atomically in the database.
- Streams the reassembled payload from `ObjectReconstructor` with HTTP headers:
  - `Content-Type: <content_type>`
  - `Content-Length: <size_bytes>`
  - `Content-Disposition: attachment; filename="<name>"`
