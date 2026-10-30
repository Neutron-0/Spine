# SPINE FORGE API SPECIFICATION

## 1. REST Conventions
- **Base URL:** `/api/v1`
- **Content Type:** `application/json`
- **Authentication:** Bearer token (`Authorization: Bearer sp_pat_<token>`) or HttpOnly session cookie.
- **Error Format:**
  ```json
  {
    "error": {
      "code": "REPOSITORY_NOT_FOUND",
      "message": "The requested repository 'alice/project' does not exist.",
      "status": 404
    }
  }
  ```

---

## 2. Core API Endpoints

### 2.1 Authentication & User
- `POST /api/v1/auth/register`: Create a new user account.
- `POST /api/v1/auth/login`: Authenticate and receive session token.
- `GET /api/v1/user/me`: Retrieve authenticated user profile.

### 2.2 Repositories
- `GET /api/v1/repos`: List repositories accessible to current user.
- `POST /api/v1/repos`: Create a new repository (`name`, `description`, `is_private`).
- `GET /api/v1/repos/{owner}/{repo}`: Get repository metadata.
- `GET /api/v1/repos/{owner}/{repo}/commits`: Retrieve commit history.
- `GET /api/v1/repos/{owner}/{repo}/tree/{commit_or_ref}`: Browse repository file tree.
- `GET /api/v1/repos/{owner}/{repo}/blob/{object_id}`: Fetch raw or highlighted file content.
- `GET /api/v1/repos/{owner}/{repo}/diff/{base}...{head}`: Compute commit or branch diff.

### 2.3 Collaboration (Issues & Pull Requests)
- `GET /api/v1/repos/{owner}/{repo}/issues`: List issues with filter parameters.
- `POST /api/v1/repos/{owner}/{repo}/issues`: Open an issue.
- `GET /api/v1/repos/{owner}/{repo}/pulls`: List pull requests.
- `POST /api/v1/repos/{owner}/{repo}/pulls`: Create pull request (`source_branch`, `target_branch`, `title`).
- `POST /api/v1/repos/{owner}/{repo}/pulls/{id}/merge`: Execute merge.
