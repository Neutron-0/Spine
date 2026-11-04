# Spine Forge — Git LFS Batch API Support

## 1. Batch Endpoint

- **Endpoint Route:**
  - `POST /:owner/:repo_git/info/lfs/objects/batch`
  - Accepts both `/{owner}/{repo}.git/info/lfs/objects/batch` and `/{owner}/{repo}/info/lfs/objects/batch`.
- **Media Types:**
  - Request: `application/vnd.git-lfs+json` (also accepts standard `application/json`).
  - Response: `application/vnd.git-lfs+json`.

---

## 2. Request Specifications

```json
{
  "operation": "upload",
  "transfers": ["basic"],
  "ref": {
    "name": "refs/heads/main"
  },
  "objects": [
    {
      "oid": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
      "size": 10485760
    }
  ],
  "hash_algo": "sha256"
}
```

### Parameter Validation Rules:
1. `operation`: Must be `"upload"` or `"download"`.
2. `transfers`: If provided, must include `"basic"`. If only unsupported transfer adapters are requested (e.g. `["custom"]`), the server responds with HTTP `422 Unprocessable Entity`:
   ```json
   {
     "message": "Unsupported transfer adapter",
     "documentation_url": "https://github.com/git-lfs/git-lfs/blob/main/docs/api/basic-transfers.md"
   }
   ```
3. `hash_algo`: If provided, must be `"sha256"`. Any other algorithm produces HTTP `422 Unprocessable Entity`.
4. `objects`: Must contain between 1 and 500 objects. Each `oid` must be a valid 64-character lowercase hex string, and `size` must be non-negative.

---

## 3. Response Semantics

### 3.1 Upload Operation

- **Existing Object (Deduplication):**
  If an object with matching SHA-256 and size already exists in the repository artifact store, `actions` is omitted:
  ```json
  {
    "oid": "e3b0c442...",
    "size": 10485760,
    "authenticated": true
  }
  ```
  The Git LFS client interprets the absence of `actions` as confirmation that the object is already on the server and skips the network upload.

- **Missing Object (Upload Actions):**
  If the object does not exist in the store, the server generates `upload` and `verify` actions:
  ```json
  {
    "oid": "e3b0c442...",
    "size": 10485760,
    "authenticated": true,
    "actions": {
      "upload": {
        "href": "https://forge.example.com/alice/demo.git/info/lfs/objects/e3b0c442...",
        "header": {
          "Authorization": "Bearer <scoped_hmac_token>"
        },
        "expires_in": 3600
      },
      "verify": {
        "href": "https://forge.example.com/alice/demo.git/info/lfs/verify",
        "header": {
          "Authorization": "Bearer <scoped_hmac_token>"
        },
        "expires_in": 3600
      }
    }
  }
  ```

### 3.2 Download Operation

- **Existing Object:**
  Returns `download` action pointing to the streaming download endpoint:
  ```json
  {
    "oid": "e3b0c442...",
    "size": 10485760,
    "authenticated": true,
    "actions": {
      "download": {
        "href": "https://forge.example.com/alice/demo.git/info/lfs/objects/e3b0c442...",
        "header": {
          "Authorization": "Bearer <scoped_hmac_token>"
        },
        "expires_in": 3600
      }
    }
  }
  ```

- **Missing Object:**
  Returns per-object error with code 404 without failing the entire batch:
  ```json
  {
    "oid": "e3b0c442...",
    "size": 10485760,
    "authenticated": true,
    "error": {
      "code": 404,
      "message": "Object does not exist"
    }
  }
  ```
