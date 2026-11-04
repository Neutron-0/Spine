# GITLAB MIGRATION GUIDE

**Project:** Spine  
**Batch Milestone:** Batch 15 — Native Git Interoperability, Repository Import/Export, Platform Migration, and Ecosystem Compatibility  
**Author:** DeepMind Advanced Agentic Coding Pair Program  
**Workspace:** `d:\Vault\Spine`

---

## 1. MIGRATING FROM GITLAB

Spine Forge natively supports migrating repositories, merge requests, issues, labels, and milestones from GitLab project export bundles.

---

## 2. PREPARING GITLAB PROJECT EXPORTS

1. In GitLab: Navigate to **Settings** -> **General** -> **Advanced** -> **Export Project**.
2. Download the resulting `.tar.gz` export bundle.
3. The bundle contains:
   - `project.bundle` (Git bundle containing all git objects, branches, and tags)
   - `issues.ndjson` or `issues.json`
   - `merge_requests.ndjson` or `merge_requests.json`
   - `labels.json`
   - `milestones.json`

---

## 3. EXECUTING THE IMPORT

### 3.1 Via Spine CLI

```bash
spine migrate run \
  --source /path/to/gitlab-export.tar.gz \
  --target /path/to/spine-repo \
  --attribution-policy ghost
```

### 3.2 Via Spine Forge Web UI

1. Open **Spine Forge** and click **Migration**.
2. Select **GitLab Export** as source kind.
3. Enter the file path of the downloaded `.tar.gz`.
4. Click **Inspect Source** to view the itemized breakdown:
   - Branches and commits
   - Merge requests translated to Spine Pull Requests
   - GitLab Issues translated to Spine Issues
5. Click **Run Migration**.

---

## 4. HANDLING GITLAB CI/CD (`.gitlab-ci.yml`)

- The `.gitlab-ci.yml` pipeline configuration file is preserved in the repository root.
- Spine Forge provides an automated migration assistant that maps GitLab jobs to Spine Workflows under `.spine/workflows/`.
- Per Spine security invariants, all translated workflows default to `is_active: false`. Repository maintainers must review and explicitly activate them.
