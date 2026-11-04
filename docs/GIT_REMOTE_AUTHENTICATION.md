# Git Remote Authentication in Spine Forge

## Overview

Spine Forge supports standard HTTP Basic Authentication across all Git Smart HTTP endpoints. Clients can authenticate using their user account credentials or dedicated Personal Access Tokens (PATs).

## Authentication Methods

### 1. Personal Access Tokens (Recommended)

Personal Access Tokens provide granular, revocable access with scoped permissions.

- **Token Format:** `sp_pat_<random_base64_or_hex>` (minimum 35 characters).
- **Git Command Example:**
  ```bash
  git clone http://<username>:<token>@forge.example.com/<owner>/<repo>.git
  ```
- **Terminal Prompt:**
  When prompted by Git:
  - `Username`: your Spine Forge username.
  - `Password`: your Personal Access Token (`sp_pat_...`).

### 2. User Password Authentication

User account passwords hashed with Argon2id can be used as the HTTP Basic password:
```bash
git clone http://<username>:<password>@forge.example.com/<owner>/<repo>.git
```

### 3. Alternative Headers

For custom automation and CI/CD pipelines, the following headers are supported:
- `Authorization: Bearer <token>`
- `X-Spine-Token: <token>`

## Access Control Matrix

| Repository Visibility | Service | Unauthenticated Client | Authenticated Reader | Authenticated Collaborator / Owner |
| :--- | :--- | :--- | :--- | :--- |
| **Public** | `git-upload-pack` (read) | `200 OK` (Anonymous clone) | `200 OK` | `200 OK` |
| **Public** | `git-receive-pack` (write) | `401 Unauthorized` | `403 Forbidden` | `200 OK` |
| **Private** | `git-upload-pack` (read) | `401 Unauthorized` | `200 OK` | `200 OK` |
| **Private** | `git-receive-pack` (write) | `401 Unauthorized` | `403 Forbidden` | `200 OK` |

When unauthenticated access is rejected, Spine Forge emits:
```http
HTTP/1.1 401 Unauthorized
WWW-Authenticate: Basic realm="Spine Forge"
```
This triggers official Git credential helpers or terminal username/password prompts automatically.
