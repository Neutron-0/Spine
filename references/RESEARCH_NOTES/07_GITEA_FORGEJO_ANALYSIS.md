# RESEARCH NOTE 07: GITEA & FORGEJO COLLABORATION PLATFORM ANALYSIS

## 1. Core Model & Philosophy
Gitea and Forgejo provide complete self-hosted developer collaboration platforms:
- **Architecture:** Monolithic Go backend communicating with Git subprocesses, backing databases (PostgreSQL, MySQL, SQLite), and local/S3 object stores.
- **Data Models:** Relational schema modeling users, organizations, repositories, branches, issues, comments, pull requests, reviews, milestones, and notifications.
- **Git Execution:** Spawns `git` CLI processes for reading commits, trees, branches, diffs, and running hooks (`pre-receive`, `post-receive`).
- **CI/CD:** Forgejo Actions uses an Act-compatible execution runner executing workflows in Docker containers.

## 2. Key Subsystems Inspected
- `services/repository/`: Repository lifecycle (creation, deletion, fork, mirror sync).
- `services/pull/`: Merge detection, three-way diff calculations, and branch conflict checking.
- `modules/storage/`: Abstraction over local filesystem, AWS S3, and Azure Blob storage for avatars, attachments, and LFS objects.

## 3. What Spine Adopts
1. **Domain-Driven Service Separation:** Explicit modules for Auth, Repositories, Pull Requests, Issues, and Webhooks.
2. **Provider-Independent Storage:** Decoupling storage nodes from relational metadata databases.

## 4. What Spine Avoids
1. **Subprocess Spawning Bottlenecks:** Gitea executes thousands of `git` CLI child processes per minute under load. Spine Forge embeds the `spine-core` and `spine-object-store` Rust libraries directly, achieving in-process zero-IPC memory speeds for repository inspection.
2. **Monolithic Spaghetti:** Spine separates the API service, distributed storage nodes, and worker queues into cleanly defined Rust micro-crates and binaries.
