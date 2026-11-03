# Branch Protection Enforcement Specification

## 1. Boundary Defense

Branch protection rules in Spine are enforced at the write pipeline boundary (`AuthoritativeRepositoryWriter::execute_write`) rather than relying purely on user-interface checks.

Even if an attacker crafts raw SPWP wire packets or bypasses the Forge UI, the authoritative write service evaluates the rule set before initiating 2PC staging.

## 2. Invariants and Protections

1. **Pull Request Requirement (`require_pull_request`)**:
   - Direct pushes to protected branches are rejected.
   - Pushes must originate through a pull request merge operation (`is_pr_merge = true`).
   - Only repository administrators with `enforce_for_admins = false` may bypass this rule (if explicitly enabled).
2. **Review Approval Threshold (`required_approvals`)**:
   - The write pipeline verifies that `pr_context.approved_count >= rule.required_approvals`.
   - If approvals are insufficient, the merge transaction is immediately aborted.
3. **Self-Approval Prevention (`prevent_self_approval`)**:
   - Approvals authored by the PR submitter (`reviewer_id == pr.author_id`) are discarded from the approval count.
4. **Change Request Blockers (`require_changes_addressed`)**:
   - If any reviewer has submitted an active `CHANGES_REQUESTED` review state that has not been dismissed or updated to `APPROVED`, the merge is rejected.
5. **Conversation Resolution Requirement (`require_conversation_resolution`)**:
   - If any review comments on the pull request remain unresolved (`is_resolved == false`), the merge is rejected.
6. **Force-Push Prevention (`allow_force_push`)**:
   - Non-fast-forward updates to protected branches are forbidden unless `allow_force_push` is explicitly active and the actor holds `Role::Admin` with a valid lease.
