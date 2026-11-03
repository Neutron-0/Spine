# Spine Forge: REST API Reference (v1)

## Base URL
All API v1 routes are served under `/api/v1`.

## Authentication Headers
Requests may authenticate using one of three mechanisms:
1. `Authorization: Bearer <token>` (Supports both `sp_sess_...` and `sp_pat_...`)
2. `X-Spine-Token: <token>` (Personal Access Token)
3. `Cookie: spine_session=<token>` (Web browser sessions)

---

## 1. Health Endpoints
- `GET /health/live`: Returns 200 OK `{ "status": "UP" }`.
- `GET /health/ready`: Returns 200 OK when database and storage root are ready.

---

## 2. Authentication Endpoints

### `POST /api/v1/auth/register`
Creates a new user account.
- **Request Body**:
  ```json
  {
    "username": "alice",
    "email": "alice@spine.build",
    "password": "Password123!"
  }
  ```
- **Response**: 201 Created
  ```json
  {
    "user": {
      "id": "usr_...",
      "username": "alice",
      "email": "alice@spine.build",
      "account_status": "ACTIVE"
    },
    "token": "sp_sess_..."
  }
  ```

### `POST /api/v1/auth/login`
Authenticates user with username or email.
- **Request Body**:
  ```json
  {
    "login": "alice",
    "password": "Password123!"
  }
  ```
- **Response**: 200 OK (includes session token and `Set-Cookie` header).

### `POST /api/v1/auth/logout`
Revokes current session.
- **Response**: 200 OK `{ "message": "Logged out" }`.

### `GET /api/v1/auth/session`
Validates current session or token.
- **Response**: 200 OK
  ```json
  {
    "authenticated": true,
    "user": { "username": "alice", "email": "alice@spine.build" },
    "session_id": "sess_..."
  }
  ```

---

## 3. Repository Management Endpoints

### `POST /api/v1/repos`
Creates a new repository.
- **Request Body**:
  ```json
  {
    "name": "distributed-kv",
    "description": "High throughput storage",
    "is_private": false,
    "durability": "ThreeCopies"
  }
  ```
- **Response**: 201 Created (provisioning status `PENDING`).

### `GET /api/v1/repos`
Lists public repositories and repositories owned by the caller.

### `GET /api/v1/repos/:owner/:name`
Retrieves repository details. If repository is private and caller is unauthorized, returns `404 Not Found`.

### `DELETE /api/v1/repos/:owner/:name`
Deletes a repository. Only authorized for repository owner.

---

## 4. Repository Inspection Endpoints

### `GET /api/v1/repos/:owner/:name/branches`
Lists all branches and current commit hashes.

### `GET /api/v1/repos/:owner/:name/tags`
Lists all tags and target commit IDs.

### `GET /api/v1/repos/:owner/:name/commits?ref=main&limit=20`
Returns commit history for specified ref or commit SHA.

### `GET /api/v1/repos/:owner/:name/tree?ref=main&path=src`
Returns entries in the specified directory tree.

### `GET /api/v1/repos/:owner/:name/blob/:path?ref=main`
Returns file content, size, binary status, and object SHA.
