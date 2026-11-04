# Pull Request Check Enforcement & Authoritative Branch Protection

## 1. Overview

In modern distributed software engineering, repository integrity requires that automated checks (unit tests, security scans, linters, and integration suites) pass before code can be merged into protected branches (e.g. `main` or `release/*`).

Spine Forge deeply integrates the **Workflow Engine**, **Commit Status Checks**, and **Branch Protection Rules** to guarantee that merges are authoritatively validated.

---

## 2. Status Check Lifecycle

```
[Developer pushes branch or opens PR]
           |
           v
[Workflow Engine triggers workflow (e.g. "CI Pipeline")]
           |
           v
[Check Result initialized in `PENDING` status for target commit OID]
           |
           v
[Branch Protection Evaluator: Merge attempts blocked with 422 Unprocessable Entity]
           |
           v
[Runner executes build and test jobs; all jobs succeed]
           |
           v
[WorkflowRun concludes with "SUCCESS"; Check Result updated to "SUCCESS"]
           |
           v
[Branch Protection Evaluator: Merge authorized and successfully executed]
```

---

## 3. Authoritative Merge Validation (`PermissionService::check_merge_allowed`)

Before an authorized repository writer can merge a Pull Request, the engine evaluates:
1. **User Write Access**: The caller must have write or admin permissions on the repository.
2. **Branch Protection Match**: The target branch (`pr.base_branch`) is evaluated against configured `branch_pattern` rules.
3. **Required Approvals**: Must meet or exceed `rule.required_approvals` (excluding self-approvals if `rule.prevent_self_approval` is active).
4. **Resolved Comments**: If `rule.require_changes_addressed` is active, all inline review comments must be resolved.
5. **Required Status Checks**:
   ```rust
   if !rule.required_status_checks.is_empty() {
       let checks = self.db.list_workflow_check_results(&repo.id, &pr.head_commit_id).await?;
       for req_check in &rule.required_status_checks {
           let check_opt = checks.iter().find(|c| c.check_name == *req_check);
           match check_opt {
               Some(c) if c.status == "SUCCESS" => {},
               Some(c) => return Err(ForgeError::UnprocessableEntity(format!(
                   "Branch protection: required check '{}' has status '{}', must be 'SUCCESS' before merging",
                   req_check, c.status
               ))),
               None => return Err(ForgeError::UnprocessableEntity(format!(
                   "Branch protection: required check '{}' has not passed for commit {}",
                   req_check, pr.head_commit_id
               ))),
           }
       }
   }
   ```

Any violation immediately aborts the merge transaction before any refs or commit trees are mutated.
