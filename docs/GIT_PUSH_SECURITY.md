# Git Push Security and Pre-Receive Enforcement

## Security Pipeline

When an authenticated client pushes objects to Spine Forge via `POST /{owner}/{repo}.git/git-receive-pack`, the payload must pass rigorous pre-receive validation before any reference is committed to the repository.

```mermaid
flowchart TD
    A["Incoming Git Push"] --> B["Unpack & Convert Objects"]
    B --> C{"Secret Scanning"}
    C -->|"Secret Detected"| D["Reject Push (ng ref) with Finding"]
    C -->|"Clean"| E{"Branch Protection Check"}
    E -->|"Direct Push Prohibited"| F["Reject Push (ng ref)"]
    E -->|"Force Push Prohibited"| G["Reject Push (ng ref)"]
    E -->|"Allowed"| H{"CAS Reference Check"}
    H -->|"Old OID Stale"| I["Reject Push (ng ref: CAS mismatch)"]
    H -->|"Matches Expected"| J["Atomic Write & Publication (ok ref)"]
```

## Pre-Receive Checks

### 1. Automated Secret Scanning

Every newly ingested blob in the packfile is inspected prior to reference publication:

- **Spine Personal Access Tokens:** Detection of strings starting with `sp_pat_` (length $\ge 35$).
- **GitHub Tokens:** Detection of strings starting with `ghp_` (length $\ge 40$).
- **AWS Access Keys:** Patterns matching `AKIA[0-9A-Z]{16}`.
- **Private Key Files:** Headers matching `-----BEGIN PRIVATE KEY-----`, `-----BEGIN RSA PRIVATE KEY-----`, or `-----BEGIN OPENSSH PRIVATE KEY-----`.

**Rejection Behavior:**
If a secret is detected:
1. The push transaction is halted.
2. In the `report-status` response, the affected ref receives `ng <ref> push rejected: secret scanning detected sensitive credentials (<type>)`.
3. An audit finding is registered in Forge's Security Center.

### 2. Branch Protection Rules

Configured branch protection rules (e.g. For `main`, `master`, or glob patterns) are strictly enforced at the transport boundary:

- **Require Pull Request:** If `require_pull_request: true`, direct pushes to the branch are rejected with:
  `push rejected: direct push to protected branch '<branch>' is prohibited; pull request required`
- **Disallow Force Push:** If `allow_force_push: false`, non-fast-forward updates (verified using topological ancestry `is_ancestor`) are rejected with:
  `push rejected: force push to protected branch '<branch>' is strictly prohibited`

### 3. Compare-And-Swap (CAS) Integrity

All ref updates specify `<old_oid>` and `<new_oid>`:
- If `<old_oid>` is non-zero, the current repository reference must match `<old_oid>`.
- If the reference on disk has advanced since the client fetched, the push is rejected with:
  `push rejected: stale reference (CAS mismatch)`
- This prevents race conditions, lost updates, and concurrent writer collisions.
