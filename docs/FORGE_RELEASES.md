# SPINE FORGE RELEASES

## 1. Overview
Spine Forge Releases provide official distribution milestones for projects, linking release notes, git tags, commit hashes, pre-release flags, and binary assets.

---

## 2. Release Data Model

A release in Spine Forge is represented by `ReleaseDto`:
```json
{
  "id": "rel_01JBMX9Z...",
  "repo_id": "rep_alpha",
  "tag_name": "v1.2.0",
  "target_commit": "a1b2c3d4e5f6...",
  "name": "Spine Release v1.2.0",
  "body": "## Changes\n- Added native LFS\n- Added Cargo registry",
  "draft": false,
  "prerelease": false,
  "author_id": "usr_dev",
  "published_at_nanos": 1793798400000000000,
  "created_at_nanos": 1793798400000000000,
  "updated_at_nanos": 1793798400000000000,
  "assets": [...]
}
```

---

## 3. Lifecycle States

1. **Draft Releases**:
   - Visible only to repository collaborators with `WRITE` or `ADMIN` permissions.
   - Tag does not need to be pushed to the repository yet.
   - Binary assets can be uploaded, tested, and modified before public exposure.
2. **Published Releases**:
   - `draft: false`. Visible to all users with `READ` access to the repository.
   - Fixed to an immutable tag and target commit hash.
   - Published timestamp permanently recorded.
3. **Pre-release Flag**:
   - Highlights alpha, beta, or release candidates without replacing the primary "Latest" release pill in the UI.

---

## 4. API Endpoints

- `GET /api/v1/repos/:owner/:name/releases`: List releases for repository.
- `POST /api/v1/repos/:owner/:name/releases`: Create draft or published release.
- `GET /api/v1/repos/:owner/:name/releases/:release_id`: Retrieve release details.
- `GET /api/v1/repos/:owner/:name/releases/tags/:tag`: Retrieve release by git tag name.
- `PATCH /api/v1/repos/:owner/:name/releases/:release_id`: Update name, body, draft, or prerelease status.
- `DELETE /api/v1/repos/:owner/:name/releases/:release_id`: Delete release and attached metadata.
