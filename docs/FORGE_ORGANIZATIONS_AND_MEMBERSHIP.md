# Spine Forge: Organizations and Multi-Tenant Membership Architecture

## 1. Overview and Core Philosophy

In Spine Forge, collaboration extends beyond individual user accounts to first-class organizational entities. An **Organization** is an enterprise-grade multi-tenant container for repositories, teams, members, and shared administrative governance.

Key tenets:
1. **Isolated Repositories:** Repositories can be owned directly by an individual user (`owner_user_id`) or by an organization (`owner_org_id`).
2. **Explicit Role Hierarchy:** Organization memberships are defined through an explicit tri-state role enum (`Owner`, `Admin`, `Member`).
3. **Multi-Tenancy Isolation:** Non-members of private organizations or private repositories cannot observe organizational existence, repositories, or membership metadata (strict `404 Not Found` semantics).
4. **Invitation Workflow:** Adding users to organizations adheres to an auditable invitation lifecycle with secure token dispatch and expiration semantics.

---

## 2. Relational Data Model

The schema for organizations, memberships, and invitations is defined in `migrations/20261103000002_forge_collaboration.sql`:

```sql
CREATE TABLE IF NOT EXISTS organizations (
    id VARCHAR(64) PRIMARY KEY,
    name VARCHAR(64) UNIQUE NOT NULL,
    normalized_slug VARCHAR(64) UNIQUE NOT NULL,
    display_name VARCHAR(128) NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    avatar_url VARCHAR(256),
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS organization_memberships (
    id VARCHAR(64) PRIMARY KEY,
    org_id VARCHAR(64) NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    user_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role VARCHAR(32) NOT NULL, -- 'Owner', 'Admin', 'Member'
    created_at_nanos BIGINT NOT NULL,
    UNIQUE (org_id, user_id)
);

CREATE TABLE IF NOT EXISTS organization_invitations (
    id VARCHAR(64) PRIMARY KEY,
    org_id VARCHAR(64) NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    inviter_user_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    invitee_email VARCHAR(256) NOT NULL,
    role VARCHAR(32) NOT NULL,
    token_hash VARCHAR(64) NOT NULL,
    status VARCHAR(32) NOT NULL DEFAULT 'PENDING',
    expires_at_nanos BIGINT NOT NULL,
    created_at_nanos BIGINT NOT NULL
);
```

### Slug Validation and Normalization
Organization names must follow strict slug conventions:
- 1 to 64 alphanumeric characters, hyphens, and underscores.
- Lowercase normalization prevents duplicate names with varying case (e.g., `Acme-Corp` vs `acme-corp`).
- Prohibited reserved slugs include `api`, `admin`, `auth`, `settings`, `explore`, and `health`.

---

## 3. Membership Roles and Privileges

| Permission Capability | Member | Admin | Owner |
|---|---|---|---|
| View public organization repositories | Yes | Yes | Yes |
| View private organization repositories (assigned) | Yes | Yes | Yes |
| Create new repositories under organization | No | Yes | Yes |
| Create and manage teams | No | Yes | Yes |
| Invite members / view pending invitations | No | Yes | Yes |
| Change member roles | No | No | Yes |
| Remove members from organization | No | Admins & Members | Anyone (except last Owner) |
| Update organization metadata / settings | No | Yes | Yes |
| Delete organization | No | No | Yes |

### Last Owner Invariant
The authorization engine enforces that an organization cannot delete its last active `Owner` membership or demote its last owner to `Admin` or `Member`. This prevents orphaned organizations.

---

## 4. API Endpoints

- `GET /api/v1/orgs`: List organizations the authenticated user belongs to.
- `POST /api/v1/orgs`: Create a new organization (caller automatically becomes an `Owner`).
- `GET /api/v1/orgs/:org_name`: Retrieve organization metadata.
- `GET /api/v1/orgs/:org_name/members`: List members and their roles.
- `POST /api/v1/orgs/:org_name/members`: Add a member or invite by username.
- `DELETE /api/v1/orgs/:org_name/members/:user_id`: Remove a member.
- `GET /api/v1/orgs/:org_name/teams`: List teams defined in the organization.
- `POST /api/v1/orgs/:org_name/teams`: Create a new team.
