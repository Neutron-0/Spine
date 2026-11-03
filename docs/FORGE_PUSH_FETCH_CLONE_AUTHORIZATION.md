# Forge Remote Authorization Model (Push, Fetch, Clone)

## 1. Permission Matrix

Access control for remote repository operations maps directly between Forge roles and native SPWP roles:

| Operation | Public Repository | Private Repository (Member) | Private Repository (Unauthorized) | Required SPWP Role |
| :--- | :--- | :--- | :--- | :--- |
| **Clone** | Allowed (Anonymous / Authenticated) | Allowed (Reader / Contributor / Owner) | **404 Not Found** | `Role::Reader` |
| **Fetch** | Allowed (Anonymous / Authenticated) | Allowed (Reader / Contributor / Owner) | **404 Not Found** | `Role::Reader` |
| **Push (Fast-Forward)** | Contributor / Admin | Contributor / Owner | **404 Not Found** | `Role::Contributor` |
| **Push (Protected / Force)** | Owner / Admin | Owner / Admin | **404 Not Found** | `Role::Admin` |

## 2. Prevention of Metadata Leakage

When an unauthenticated or unauthorized caller requests access to a private repository:
- The Forge API endpoint returns `404 Not Found` (rather than `401 Unauthorized` or `403 Forbidden`).
- This prevents attackers from enumerating repository names, testing the existence of organizations or private projects, or probing permission boundaries.

## 3. Cryptographic Ticket Validation

- The capability ticket contains the repository's native ID (`native_repo_id`), bound to the actor's ID and role.
- During `REPO_OPEN`, the SPWP daemon cross-checks that the requested repository path matches `ticket.native_repo_id`.
- If a client presents a valid ticket for `repo_A` while attempting to access `repo_B`, the daemon returns `403 Forbidden` (`REPO_OPEN_REJECTED`).
