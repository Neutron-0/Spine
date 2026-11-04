# GITHUB MIGRATION GUIDE

**Project:** Spine  
**Batch Milestone:** Batch 15 — Native Git Interoperability, Repository Import/Export, Platform Migration, and Ecosystem Compatibility  
**Author:** DeepMind Advanced Agentic Coding Pair Program  
**Workspace:** `d:\Vault\Spine`

---

## 1. MIGRATING FROM GITHUB

Spine Forge provides a seamless migration path from GitHub export archives or cloned repositories into Spine Forge. The platform migrates both code history and project collaboration records (issues, pull requests, comments, labels, milestones, and releases).

---

## 2. PREPARING GITHUB EXPORT ARCHIVES

GitHub provides migration exports via the GitHub Migration API or export tarballs:
1. Export your GitHub repository archive (`.tar.gz` or `.zip`).
2. Alternatively, clone the GitHub repository with all branches:
   ```bash
   git clone --mirror https://github.com/owner/repo.git
   ```
3. If using metadata JSON files (issues, pull requests, releases), place them into a metadata folder or archive matching:
   - `issues.json`
   - `pull_requests.json`
   - `labels.json`
   - `milestones.json`
   - `releases.json`
   - `users.json`

---

## 3. RUNNING THE MIGRATION

### 3.1 Via Spine CLI

```bash
spine migrate run \
  --source /path/to/github-archive.tar.gz \
  --target /path/to/spine-repo \
  --attribution-policy ghost \
  --deactivate-workflows
```

### 3.2 Via Spine Forge Web UI

1. Open **Spine Forge** -> **Migration**.
2. Select **GitHub Archive** as the source kind.
3. Upload or point to the export path.
4. Review the **Attribution Policy**:
   - `Ghost User Mapping` (Default): Maps unknown users to `ghost-<username>` tombstone accounts.
   - `Email Alias Mapping`: Associates external authors with registered Spine Forge users if emails match.
5. Click **Run Migration**.

---

## 4. SECURITY & WORKFLOW SAFETY

### 4.1 Inactive Workflow Enforcement (`is_active: false`)
Any GitHub Actions workflows found in `.github/workflows/*.yml` or `.yaml` are automatically imported into `.spine/workflows/` but are marked as **inactive**:
```yaml
# Inactive on import
is_active: false
```
*Why?* Malicious actors could stage malicious CI workflows in public repositories intended to exfiltrate tokens or execute cryptominers upon migration. Spine requires an authorized repository administrator to inspect the workflow syntax and manually activate it.

### 4.2 Safe User Attribution
Imported external users never receive organization or repository administrative privileges. They are recorded as attribution tombstones so issue authorship, PR commentary, and commit history remain historically accurate without introducing privilege escalation vectors.
