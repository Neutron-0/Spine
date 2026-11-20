# API Authorization & Tenant-Isolation Matrix

**Milestone:** Production Deployment Closure & Complete Authorization Audit  
**Classification:** Security Architecture & Authorization Specification  
**Workspace:** `d:\Vault\Spine`

---

## 1. High-Level Role & Resource Access Matrix

| API Resource / Endpoint | Anonymous User | Authenticated Non-Member | Repo Reader (`READ`) | Repo Writer (`WRITE`) | Repo Admin (`ADMIN`) | Org Admin | Registered Runner |
|---|---|---|---|---|---|---|---|
| **Public Repo (Read / Clone)** | ✅ Permitted | ✅ Permitted | ✅ Permitted | ✅ Permitted | ✅ Permitted | ✅ Permitted | ❌ (Needs token) |
| **Private Repo (Read / Clone)** | ❌ 401 Unauthorized | ❌ 404 Hidden | ✅ Permitted | ✅ Permitted | ✅ Permitted | ✅ Permitted | ❌ (Needs ticket) |
| **Branch Push / Update** | ❌ 401 Unauthorized | ❌ 403 / 404 | ❌ 403 Forbidden | ✅ Permitted (if unprotected) | ✅ Permitted | ✅ Permitted | ❌ 403 Forbidden |
| **Protected Branch Push** | ❌ 401 Unauthorized | ❌ 403 / 404 | ❌ 403 Forbidden | ❌ 403 Blocked (Needs PR) | ❌ (Needs PR/Admin) | ❌ (Needs PR) | ❌ 403 Forbidden |
| **Pull Request (Open / Comment)**| ❌ 401 Unauthorized | ✅ Permitted (Public) | ✅ Permitted | ✅ Permitted | ✅ Permitted | ✅ Permitted | ❌ 403 Forbidden |
| **Pull Request (Merge)** | ❌ 401 Unauthorized | ❌ 403 / 404 | ❌ 403 Forbidden | ✅ Permitted (Checks pass) | ✅ Permitted | ✅ Permitted | ❌ 403 Forbidden |
| **PR Review (Create / Comment)** | ❌ 401 Unauthorized | ❌ 403 / 404 | ✅ Permitted | ✅ Permitted | ✅ Permitted | ✅ Permitted | ❌ 403 Forbidden |
| **PR Review Dismissal** | ❌ 401 Unauthorized | ❌ 403 / 404 | ❌ 403 Forbidden | ❌ 403 Forbidden | ✅ Permitted (Scoped to PR) | ✅ Permitted | ❌ 403 Forbidden |
| **Discussions (Read / Categories)** | Public only | Public only | ✅ Permitted | ✅ Permitted | ✅ Permitted | ✅ Permitted | ❌ 403 Forbidden |
| **Discussions (Create / Reply)**| ❌ 401 Unauthorized | Public only | ✅ Permitted | ✅ Permitted | ✅ Permitted | ✅ Permitted | ❌ 403 Forbidden |
| **Mark Accepted Answer** | ❌ 401 Unauthorized | ❌ 403 / 404 | ❌ 403 (Unless OP) | ✅ Permitted | ✅ Permitted | ✅ Permitted | ❌ 403 Forbidden |
| **LFS Upload / Download** | Download only (Public) | Download only (Public) | Download only | ✅ Read / Write | ✅ Read / Write | ✅ Read / Write | ❌ 403 Forbidden |
| **LFS Lock / Unlock** | ❌ 401 Unauthorized | ❌ 403 / 404 | ❌ 403 Forbidden | ✅ Lock / Own Unlock | ✅ Force Unlock | ✅ Force Unlock | ❌ 403 Forbidden |
| **Cargo Package Publish** | ❌ 401 Unauthorized | ❌ 403 Forbidden | ❌ 403 Forbidden | ❌ (Unless package owner) | ✅ (Package owner) | ✅ (Package owner) | ❌ 403 Forbidden |
| **Cargo Package Yank/Unyank** | ❌ 401 Unauthorized | ❌ 403 Forbidden | ❌ 403 Forbidden | ❌ (Unless package owner) | ✅ (Package owner) | ✅ (Package owner) | ❌ 403 Forbidden |
| **Releases & Assets (Read)** | Public only | Public only | ✅ Permitted | ✅ Permitted | ✅ Permitted | ✅ Permitted | ❌ 403 Forbidden |
| **Releases & Assets (Upload/Mutate)** | ❌ 401 Unauthorized | ❌ 403 / 404 | ❌ 403 Forbidden | ❌ 403 Forbidden | ✅ Permitted (Scoped to repo) | ✅ Permitted | ❌ 403 Forbidden |
| **Webhooks (View / Mutate)** | ❌ 401 Unauthorized | ❌ 403 / 404 | ❌ 403 Forbidden | ❌ 403 Forbidden | ✅ Permitted (Repo/Org owner) | ✅ Permitted | ❌ 403 Forbidden |
| **Branch Protection Rules** | ❌ 401 Unauthorized | ❌ 403 / 404 | ❌ 403 Forbidden | ❌ 403 Forbidden | ✅ Permitted (Scoped to repo) | ✅ Permitted | ❌ 403 Forbidden |
| **Workflow Run (Trigger / View)**| View only (Public) | View only (Public) | View only | ✅ Trigger / View | ✅ Trigger / View | ✅ Trigger / View | ❌ 403 Forbidden |
| **Workflow Job (Acquire / Log)**| ❌ 401 Unauthorized | ❌ 403 Forbidden | ❌ 403 Forbidden | ❌ 403 Forbidden | ❌ 403 Forbidden | ❌ 403 Forbidden | ✅ Permitted (Scoped lease) |
| **Workflow Secrets (View / Set)**| ❌ 401 Unauthorized | ❌ 403 / 404 | ❌ 403 Forbidden | ❌ 403 Forbidden | ✅ Permitted | ✅ Permitted | ❌ (Masked in logs) |
| **Security Center (Scans / Findings)**| ❌ 401 Unauthorized | ❌ 403 / 404 | ✅ View only | ✅ Scan / Remediate | ✅ Full Admin | ✅ Full Admin | ❌ 403 Forbidden |
| **SBOM Export (SPDX / CycloneDX)**| ❌ 401 Unauthorized | ❌ 403 / 404 | ✅ Permitted | ✅ Permitted | ✅ Permitted | ✅ Permitted | ❌ 403 Forbidden |
| **Repository Discovery (Topics/Langs)**| Public only | Public only | ✅ Permitted | ✅ Permitted | ✅ Permitted | ✅ Permitted | ❌ 403 Forbidden |

---

## 2. Route-Level Permission & Tenancy Enforcement Matrix

The table below catalogs every API route module, the required caller privilege, and the tenancy isolation mechanism enforced at the handler boundary:

### 2.1 Repositories & Git Access (`routes_repos.rs`, `routes_git_http.rs`)
| Method | Route | Handler | Required Privilege | Tenancy Enforcement Mechanism |
| :--- | :--- | :--- | :--- | :--- |
| `POST` | `/api/v1/repos` | `create_repository` | Authenticated | User ownership bound to `caller.id`. |
| `GET` | `/api/v1/repos` | `list_repositories` | Optional User | Filtered: public repos + private repos caller has `READ` on. |
| `GET` | `/api/v1/repos/:owner/:repo` | `get_repository` | Optional User | Private repos return 404 unless caller has `READ`. |
| `DELETE`| `/api/v1/repos/:owner/:repo` | `delete_repository` | Repo Owner | Caller must match repo owner ID. |
| `GET` | `/api/v1/repos/:owner/:repo/branches` | `list_repo_branches` | Optional User | Gated by `get_repository` (404 on private). |
| `GET` | `/api/v1/repos/:owner/:repo/tags` | `list_repo_tags` | Optional User | Gated by `get_repository` (404 on private). |
| `GET` | `/api/v1/repos/:owner/:repo/commits` | `list_repo_commits` | Optional User | Gated by `get_repository` (404 on private). |
| `GET` | `/api/v1/repos/:owner/:repo/tree/*` | `get_repo_tree` | Optional User | Gated by `get_repository` (404 on private). |
| `GET` | `/api/v1/repos/:owner/:repo/blob/*` | `get_repo_blob` | Optional User | Gated by `get_repository` (404 on private). |
| `POST` | `/api/v1/repos/:owner/:repo/ticket` | `issue_capability_ticket`| Authenticated | Permissions checked against requested operation (`read`/`write`). |
| `GET` | `/api/v1/repos/:owner/:repo/keys` | `list_deploy_keys` | Repo Writer | `check_write_access` on repo. |
| `POST` | `/api/v1/repos/:owner/:repo/keys` | `create_deploy_key` | Repo Admin | `check_admin_access` on repo. |
| `DELETE`| `/api/v1/repos/:owner/:repo/keys/:id`| `delete_deploy_key` | Repo Admin | `revoke_deploy_key(&key_id, &repo.id)` verified against repo ID. |
| `GET` | `/:owner/:repo/info/refs` | `git_info_refs` | HTTP Basic / Anon | Service checked; private repos require read authentication. |
| `POST` | `/:owner/:repo/git-upload-pack` | `git_upload_pack` | HTTP Basic / Anon | Private repos require read authentication. |
| `POST` | `/:owner/:repo/git-receive-pack` | `git_receive_pack` | HTTP Basic | Requires authenticated writer; branch protections enforced. |

### 2.2 Pull Requests & Code Reviews (`routes_pulls.rs`, `routes_reviews.rs`)
| Method | Route | Handler | Required Privilege | Tenancy Enforcement Mechanism |
| :--- | :--- | :--- | :--- | :--- |
| `GET` | `/api/v1/repos/:owner/:repo/pulls` | `list_pull_requests` | Optional User | Queries `WHERE repo_id = $1` after `get_repository`. |
| `POST` | `/api/v1/repos/:owner/:repo/pulls` | `create_pull_request` | Repo Reader | Verified via `get_repository`. Author bound to `caller.id`. |
| `GET` | `/api/v1/repos/:owner/:repo/pulls/:number` | `get_pull_request` | Optional User | Queries `(repo.id, number)` composite primary key. |
| `PATCH`| `/api/v1/repos/:owner/:repo/pulls/:number` | `update_pull_request` | PR Author / Writer | Caller must be author or collaborator on `repo.id`. |
| `POST` | `/api/v1/repos/:owner/:repo/pulls/:number/merge` | `execute_pull_request_merge`| Repo Writer | `check_merge_allowed` verifying checks, reviews, and branch rules. |
| `GET` | `/api/v1/repos/:owner/:repo/pulls/:number/reviews` | `list_reviews` | Optional User | Scoped to PR ID belonging to `repo.id`. |
| `POST` | `/api/v1/repos/:owner/:repo/pulls/:number/reviews` | `create_review` | Repo Reader | Scoped to PR ID. Reviewer bound to `caller.id`. |
| `POST` | `/api/v1/repos/:owner/:repo/pulls/:number/reviews/:id/dismiss` | `dismiss_review` | Repo Admin | Verified `review_id` belongs to `pr.id` on `repo.id`. |
| `POST` | `/api/v1/repos/:owner/:repo/pulls/:number/reviews/comments/:id/resolve` | `resolve_review_comment`| Repo Writer | Verified `comment_id` belongs to `pr.id` on `repo.id`. |

### 2.3 Issues, Labels, Milestones & Protection (`routes_issues.rs`, `routes_protection.rs`)
| Method | Route | Handler | Required Privilege | Tenancy Enforcement Mechanism |
| :--- | :--- | :--- | :--- | :--- |
| `GET` | `/api/v1/repos/:owner/:repo/issues` | `list_issues` | Optional User | Scoped to `repo.id`. |
| `POST` | `/api/v1/repos/:owner/:repo/issues` | `create_issue` | Repo Reader | Verified via `get_repository`. |
| `DELETE`| `/api/v1/repos/:owner/:repo/labels/:id` | `delete_label` | Repo Admin | Verified `label_id` exists in `list_labels_for_repo(&repo.id)`. |
| `PATCH`| `/api/v1/repos/:owner/:repo/milestones/:id`| `update_milestone` | Repo Admin | Verified `milestone.repo_id == repo.id`. |
| `DELETE`| `/api/v1/repos/:owner/:repo/milestones/:id`| `delete_milestone` | Repo Admin | Verified `milestone.repo_id == repo.id`. |
| `DELETE`| `/api/v1/repos/:owner/:repo/branches/:b/protection` | `delete_branch_protection` | Repo Admin | Verified `rule.repo_id == repo.id`. |

### 2.4 Releases, Packages & Generic Artifacts (`routes_releases.rs`, `routes_packages.rs`, `routes_artifacts.rs`)
| Method | Route | Handler | Required Privilege | Tenancy Enforcement Mechanism |
| :--- | :--- | :--- | :--- | :--- |
| `GET` | `/api/v1/repos/:owner/:repo/releases` | `list_releases` | Optional User | Gated by `get_repository` read check. |
| `GET` | `/api/v1/repos/:owner/:repo/releases/:id` | `get_release` | Optional User | Gated by repo read check and `rel.repo_id == repo.id`. |
| `GET` | `/api/v1/repos/:owner/:repo/releases/assets/:id` | `download_release_asset` | Optional User | Gated by repo read check and `asset.release_id == rel.id`. |
| `DELETE`| `/api/v1/repos/:owner/:repo/releases/:id` | `delete_release` | Repo Admin | Verified `rel.repo_id == repo.id`. |
| `PUT` | `/api/v1/crates/new` | `cargo_publish` | Authenticated | First version binds owner; new version requires `pkg.owner_id == caller.id`. |
| `DELETE`| `/api/v1/crates/:name/:vers/yank` | `cargo_yank` | Package Owner | Verified `pkg.owner_id == caller.id`. |
| `PUT` | `/api/v1/crates/:name/:vers/unyank` | `cargo_unyank` | Package Owner | Verified `pkg.owner_id == caller.id`. |
| `GET` | `/api/v1/artifacts/:namespace/:artifact/versions` | `list_artifact_versions` | Namespace Owner / Public | If `is_private`, requires `ns.owner_id == caller.id`. |
| `GET` | `/api/v1/artifacts/:namespace/:artifact/versions/:v/download` | `download_artifact_version` | Namespace Owner / Public | If `is_private`, requires `ns.owner_id == caller.id`. |

### 2.5 Webhooks & Security Center (`routes_webhooks.rs`, `routes_security.rs`)
| Method | Route | Handler | Required Privilege | Tenancy Enforcement Mechanism |
| :--- | :--- | :--- | :--- | :--- |
| `POST` | `/api/v1/repos/:owner/:repo/webhooks` | `create_repo_webhook` | Repo Admin | `check_admin_access` on target repo. |
| `GET` | `/api/v1/webhooks/:id` | `get_webhook` | Repo/Org Admin | Verified caller is admin of `sub.owner_id` (repo or org). |
| `DELETE`| `/api/v1/webhooks/:id` | `delete_webhook` | Repo/Org Admin | Verified caller is admin of `sub.owner_id` before deleting. |
| `GET` | `/api/v1/webhooks/:id/deliveries` | `list_webhook_deliveries`| Repo/Org Admin | Verified caller is admin of `sub.owner_id` before listing logs. |
| `POST` | `/api/v1/repos/:owner/:repo/security/secrets/findings/:id/remediate` | `remediate_secret_finding` | Repo Writer | Verified `finding_id` belongs to `repo.id`. |
| `POST` | `/api/v1/repos/:owner/:repo/security/vulnerabilities/:id/remediate` | `remediate_vulnerability` | Repo Writer | Verified `vuln_id` belongs to `repo.id`. |

### 2.6 Discussions & Discovery (`routes_discussions.rs`, `routes_discovery.rs`)
| Method | Route | Handler | Required Privilege | Tenancy Enforcement Mechanism |
| :--- | :--- | :--- | :--- | :--- |
| `GET` | `/api/v1/repos/:owner/:repo/discussions/categories` | `list_categories` | Repo Reader | Gated by `require_repo_read` (404 on private). |
| `POST` | `/api/v1/repos/:owner/:repo/discussions/:num/comments/:id/reactions` | `toggle_comment_reaction` | Repo Reader | Verified comment belongs to discussion on `repo.id`. |
| `GET` | `/api/v1/repos/:owner/:repo/topics` | `list_repo_topics` | Repo Reader | Gated by `require_repo_read` (404 on private). |
| `POST` | `/api/v1/repos/:owner/:repo/topics` | `set_repo_topics` | Repo Writer | `require_repo_write` on target repo. |
| `GET` | `/api/v1/repos/:owner/:repo/languages` | `list_repo_languages` | Repo Reader | Gated by `require_repo_read` (404 on private). |
| `POST` | `/api/v1/repos/:owner/:repo/languages/analyze` | `analyze_repo_languages` | Repo Writer | `require_repo_write` on target repo. |

---

## 3. Horizontal Privilege Escalation Prevention Principles

1. **Existence Obfuscation (404 vs 403):** All private repository endpoints return HTTP 404 Not Found to unauthorized callers rather than HTTP 403 Forbidden, mitigating resource enumeration and metadata snooping.
2. **Composite Tenant Scoping:** Every database mutation or lookup involving child entities (pull request reviews, comments, labels, milestones, findings, webhooks) filters by `(repo_id, entity_id)` or verifies parent tenancy before execution. Global UUIDs alone are never trusted.
3. **Write Barrier Fencing:** All state-modifying HTTP endpoints acquire a `ConsistencyBarrier` write permit, ensuring atomicity and blocking concurrent writes during out-of-band backup snapshots.
