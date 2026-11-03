# Spine Forge: Collaboration REST API Reference (Batch 09)

## 1. Overview

This document provides a comprehensive reference of all REST API endpoints added in Batch 09 for organizations, teams, issues, milestones, labels, pull requests, reviews, inline comments, branch protection, notifications, and activity feeds.

All endpoints are served under `/api/v1/` and accept/return JSON. Authentication is performed via `Authorization: Bearer <session_or_pat_token>` or session cookies.

---

## 2. Endpoints Summary

### Organizations & Teams
| Method | Endpoint | Description | Auth Required |
|---|---|---|---|
| `GET` | `/api/v1/orgs` | List organizations for caller | Yes |
| `POST` | `/api/v1/orgs` | Create an organization | Yes |
| `GET` | `/api/v1/orgs/:org_name` | Get organization metadata | Yes (or public) |
| `GET` | `/api/v1/orgs/:org_name/members` | List organization members | Yes |
| `POST` | `/api/v1/orgs/:org_name/members` | Add member or invite to organization | Org Admin/Owner |
| `DELETE` | `/api/v1/orgs/:org_name/members/:user_id` | Remove organization member | Org Admin/Owner |
| `GET` | `/api/v1/orgs/:org_name/teams` | List organization teams | Org Member |
| `POST` | `/api/v1/orgs/:org_name/teams` | Create a team in organization | Org Admin/Owner |
| `POST` | `/api/v1/orgs/:org_name/teams/:slug/members` | Add user to team | Org Admin/Owner |
| `DELETE` | `/api/v1/orgs/:org_name/teams/:slug/members/:user_id` | Remove user from team | Org Admin/Owner |

### Issues, Labels & Milestones
| Method | Endpoint | Description | Auth Required |
|---|---|---|---|
| `GET` | `/api/v1/repos/:owner/:repo/issues` | List issues (status filter: `OPEN`/`CLOSED`) | Repo Read |
| `POST` | `/api/v1/repos/:owner/:repo/issues` | Create an issue | Repo Read |
| `GET` | `/api/v1/repos/:owner/:repo/issues/:number` | Get issue details with comments | Repo Read |
| `PATCH` | `/api/v1/repos/:owner/:repo/issues/:number` | Update issue title, body, status, assignees | Repo Write/Author |
| `GET` | `/api/v1/repos/:owner/:repo/issues/:number/comments`| List issue comments | Repo Read |
| `POST` | `/api/v1/repos/:owner/:repo/issues/:number/comments`| Post a comment to an issue | Repo Read |
| `GET` | `/api/v1/repos/:owner/:repo/labels` | List repository labels | Repo Read |
| `POST` | `/api/v1/repos/:owner/:repo/labels` | Create a repository label | Repo Maintain/Admin |
| `GET` | `/api/v1/repos/:owner/:repo/milestones` | List repository milestones | Repo Read |
| `POST` | `/api/v1/repos/:owner/:repo/milestones` | Create a repository milestone | Repo Maintain/Admin |

### Pull Requests, Diffs, Reviews & Merges
| Method | Endpoint | Description | Auth Required |
|---|---|---|---|
| `GET` | `/api/v1/repos/:owner/:repo/pulls` | List pull requests (`OPEN`/`CLOSED`/`MERGED`) | Repo Read |
| `POST` | `/api/v1/repos/:owner/:repo/pulls` | Propose a new pull request | Repo Write |
| `GET` | `/api/v1/repos/:owner/:repo/pulls/:number` | Get pull request details | Repo Read |
| `PATCH` | `/api/v1/repos/:owner/:repo/pulls/:number` | Update title, body, or state | Repo Write/Author |
| `GET` | `/api/v1/repos/:owner/:repo/pulls/:number/diff` | Get three-dot native unified diff | Repo Read |
| `GET` | `/api/v1/repos/:owner/:repo/pulls/:number/mergeability` | Evaluate mergeability & conflicts | Repo Read |
| `POST` | `/api/v1/repos/:owner/:repo/pulls/:number/merge` | Execute atomic 3-way merge | Repo Write |
| `GET` | `/api/v1/repos/:owner/:repo/pulls/:number/reviews` | List submitted code reviews | Repo Read |
| `POST` | `/api/v1/repos/:owner/:repo/pulls/:number/reviews` | Submit code review (Approve/Request Changes) | Repo Write |
| `GET` | `/api/v1/repos/:owner/:repo/pulls/:number/comments`| List inline review comments | Repo Read |
| `POST` | `/api/v1/repos/:owner/:repo/pulls/:number/comments`| Post inline review comment on diff hunk | Repo Write |
| `PATCH` | `/api/v1/repos/:owner/:repo/pulls/:number/comments/:id/resolve` | Toggle resolved status | Repo Write |

### Branch Protection Rules
| Method | Endpoint | Description | Auth Required |
|---|---|---|---|
| `GET` | `/api/v1/repos/:owner/:repo/protection` | List branch protection rules | Repo Read |
| `POST` | `/api/v1/repos/:owner/:repo/protection` | Create/update branch protection rule | Repo Admin |
| `DELETE` | `/api/v1/repos/:owner/:repo/protection/:rule_id`| Delete branch protection rule | Repo Admin |

### Notifications & Activity
| Method | Endpoint | Description | Auth Required |
|---|---|---|---|
| `GET` | `/api/v1/notifications` | List user notifications (`unread_only=true`) | Yes |
| `PATCH` | `/api/v1/notifications/:id/read` | Mark single notification as read | Yes |
| `POST` | `/api/v1/notifications/read-all` | Mark all user notifications as read | Yes |
| `GET` | `/api/v1/repos/:owner/:repo/activity` | List repository activity events | Repo Read |
