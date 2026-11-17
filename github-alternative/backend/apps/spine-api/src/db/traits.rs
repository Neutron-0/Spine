use crate::core::error::ForgeError;
use crate::core::types::*;
use async_trait::async_trait;

#[async_trait]
pub trait Database: Send + Sync {
    // -------------------------------------------------------------------------
    // Liveness / Connectivity
    // -------------------------------------------------------------------------
    async fn ping(&self) -> Result<(), ForgeError> {
        Ok(())
    }

    async fn get_restore_generation(&self) -> Result<Option<String>, ForgeError> {
        Ok(None)
    }

    // -------------------------------------------------------------------------
    // Users
    // -------------------------------------------------------------------------
    async fn create_user(&self, user: User) -> Result<User, ForgeError>;
    async fn get_user_by_id(&self, id: &str) -> Result<Option<User>, ForgeError>;
    async fn get_user_by_username(&self, username: &str) -> Result<Option<User>, ForgeError>;
    async fn get_user_by_email(&self, email: &str) -> Result<Option<User>, ForgeError>;
    async fn update_user_last_login(&self, user_id: &str, now_nanos: i64)
        -> Result<(), ForgeError>;
    async fn update_user_password(
        &self,
        user_id: &str,
        new_hash: &str,
        now_nanos: i64,
    ) -> Result<(), ForgeError>;

    // -------------------------------------------------------------------------
    // Sessions
    // -------------------------------------------------------------------------
    async fn create_session(&self, session: UserSession) -> Result<UserSession, ForgeError>;
    async fn get_session_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<UserSession>, ForgeError>;
    async fn revoke_session(&self, session_id: &str) -> Result<(), ForgeError>;
    async fn revoke_all_user_sessions(&self, user_id: &str) -> Result<(), ForgeError>;
    async fn list_user_sessions(&self, user_id: &str) -> Result<Vec<UserSession>, ForgeError>;

    // -------------------------------------------------------------------------
    // Verification & Reset Tokens
    // -------------------------------------------------------------------------
    async fn create_email_token(
        &self,
        token_hash: &str,
        user_id: &str,
        expires_at_nanos: i64,
    ) -> Result<(), ForgeError>;
    async fn consume_email_token(
        &self,
        token_hash: &str,
        now_nanos: i64,
    ) -> Result<Option<String>, ForgeError>;

    async fn create_reset_token(
        &self,
        token_hash: &str,
        user_id: &str,
        expires_at_nanos: i64,
    ) -> Result<(), ForgeError>;
    async fn consume_reset_token(
        &self,
        token_hash: &str,
        now_nanos: i64,
    ) -> Result<Option<String>, ForgeError>;

    // -------------------------------------------------------------------------
    // Personal Access Tokens
    // -------------------------------------------------------------------------
    async fn create_pat(&self, pat: PersonalAccessToken)
        -> Result<PersonalAccessToken, ForgeError>;
    async fn get_pat_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<PersonalAccessToken>, ForgeError>;
    async fn update_pat_last_used(&self, pat_id: &str, now_nanos: i64) -> Result<(), ForgeError>;
    async fn list_user_pats(&self, user_id: &str) -> Result<Vec<PersonalAccessToken>, ForgeError>;
    async fn revoke_pat(&self, pat_id: &str, user_id: &str) -> Result<(), ForgeError>;

    // -------------------------------------------------------------------------
    // Repositories
    // -------------------------------------------------------------------------
    async fn create_repository(&self, repo: ForgeRepository)
        -> Result<ForgeRepository, ForgeError>;
    async fn get_repository_by_id(&self, id: &str) -> Result<Option<ForgeRepository>, ForgeError>;
    async fn get_repository_by_owner_name(
        &self,
        owner: &str,
        name: &str,
    ) -> Result<Option<ForgeRepository>, ForgeError>;
    async fn list_repositories_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<ForgeRepository>, ForgeError>;
    async fn list_repositories_for_org(
        &self,
        org_id: &str,
    ) -> Result<Vec<ForgeRepository>, ForgeError>;
    async fn list_public_repositories(&self) -> Result<Vec<ForgeRepository>, ForgeError>;
    async fn update_repository_status(
        &self,
        repo_id: &str,
        status: ProvisioningStatus,
        now_nanos: i64,
    ) -> Result<(), ForgeError>;
    async fn update_repository_owner_org(
        &self,
        repo_id: &str,
        org_id: &str,
        org_name: &str,
    ) -> Result<(), ForgeError>;
    async fn delete_repository(&self, repo_id: &str) -> Result<(), ForgeError>;

    // -------------------------------------------------------------------------
    // Permissions (Direct Collaborator)
    // -------------------------------------------------------------------------
    async fn create_permission(
        &self,
        perm: RepositoryPermission,
    ) -> Result<RepositoryPermission, ForgeError>;
    async fn get_permission(
        &self,
        repo_id: &str,
        user_id: &str,
    ) -> Result<Option<PermissionRole>, ForgeError>;
    async fn list_permissions_for_repo(
        &self,
        repo_id: &str,
    ) -> Result<Vec<RepositoryPermission>, ForgeError>;
    async fn delete_permission(&self, repo_id: &str, user_id: &str) -> Result<(), ForgeError>;

    // -------------------------------------------------------------------------
    // Provisioning Jobs (Outbox)
    // -------------------------------------------------------------------------
    async fn create_provisioning_job(
        &self,
        job: ProvisioningJob,
    ) -> Result<ProvisioningJob, ForgeError>;
    async fn get_next_pending_job(&self) -> Result<Option<ProvisioningJob>, ForgeError>;
    async fn update_job_status(
        &self,
        job_id: &str,
        status: ProvisioningStatus,
        last_error: Option<String>,
        now_nanos: i64,
    ) -> Result<(), ForgeError>;

    // -------------------------------------------------------------------------
    // Organizations & Membership (Batch 09)
    // -------------------------------------------------------------------------
    async fn create_organization(&self, org: Organization) -> Result<Organization, ForgeError>;
    async fn get_organization_by_id(&self, id: &str) -> Result<Option<Organization>, ForgeError>;
    async fn get_organization_by_slug(
        &self,
        slug: &str,
    ) -> Result<Option<Organization>, ForgeError>;
    async fn list_organizations_for_user(
        &self,
        user_id: &str,
    ) -> Result<Vec<Organization>, ForgeError>;
    async fn update_organization(&self, org: Organization) -> Result<Organization, ForgeError>;
    async fn delete_organization(&self, org_id: &str) -> Result<(), ForgeError>;

    async fn add_organization_member(
        &self,
        membership: OrganizationMembership,
    ) -> Result<(), ForgeError>;
    async fn get_organization_membership(
        &self,
        org_id: &str,
        user_id: &str,
    ) -> Result<Option<OrganizationMembership>, ForgeError>;
    async fn list_organization_members(
        &self,
        org_id: &str,
    ) -> Result<Vec<OrganizationMembership>, ForgeError>;
    async fn remove_organization_member(
        &self,
        org_id: &str,
        user_id: &str,
    ) -> Result<(), ForgeError>;
    async fn update_organization_member_role(
        &self,
        org_id: &str,
        user_id: &str,
        role: OrganizationRole,
    ) -> Result<(), ForgeError>;

    async fn create_organization_invitation(
        &self,
        inv: OrganizationInvitation,
    ) -> Result<OrganizationInvitation, ForgeError>;
    async fn get_organization_invitation_by_token(
        &self,
        token_hash: &str,
    ) -> Result<Option<OrganizationInvitation>, ForgeError>;
    async fn list_organization_invitations(
        &self,
        org_id: &str,
    ) -> Result<Vec<OrganizationInvitation>, ForgeError>;
    async fn update_organization_invitation_status(
        &self,
        id: &str,
        status: &str,
        now_nanos: i64,
    ) -> Result<(), ForgeError>;

    // -------------------------------------------------------------------------
    // Teams & Team Permissions (Batch 09)
    // -------------------------------------------------------------------------
    async fn create_team(&self, team: Team) -> Result<Team, ForgeError>;
    async fn get_team_by_id(&self, id: &str) -> Result<Option<Team>, ForgeError>;
    async fn get_team_by_slug(&self, org_id: &str, slug: &str) -> Result<Option<Team>, ForgeError>;
    async fn list_teams_for_org(&self, org_id: &str) -> Result<Vec<Team>, ForgeError>;
    async fn delete_team(&self, team_id: &str) -> Result<(), ForgeError>;

    async fn add_team_member(&self, membership: TeamMembership) -> Result<(), ForgeError>;
    async fn remove_team_member(&self, team_id: &str, user_id: &str) -> Result<(), ForgeError>;
    async fn list_team_members(&self, team_id: &str) -> Result<Vec<TeamMembership>, ForgeError>;

    async fn set_team_repository_permission(
        &self,
        perm: TeamRepositoryPermission,
    ) -> Result<(), ForgeError>;
    async fn get_team_repository_permission(
        &self,
        team_id: &str,
        repo_id: &str,
    ) -> Result<Option<TeamPermissionLevel>, ForgeError>;
    async fn list_team_repository_permissions_for_repo(
        &self,
        repo_id: &str,
    ) -> Result<Vec<TeamRepositoryPermission>, ForgeError>;
    async fn list_teams_for_user_in_org(
        &self,
        org_id: &str,
        user_id: &str,
    ) -> Result<Vec<Team>, ForgeError>;

    // -------------------------------------------------------------------------
    // Milestones & Labels (Batch 09)
    // -------------------------------------------------------------------------
    async fn create_milestone(&self, milestone: Milestone) -> Result<Milestone, ForgeError>;
    async fn get_milestone_by_id(&self, id: &str) -> Result<Option<Milestone>, ForgeError>;
    async fn list_milestones_for_repo(&self, repo_id: &str) -> Result<Vec<Milestone>, ForgeError>;
    async fn update_milestone(&self, milestone: Milestone) -> Result<Milestone, ForgeError>;
    async fn delete_milestone(&self, id: &str) -> Result<(), ForgeError>;

    async fn create_label(&self, label: RepositoryLabel) -> Result<RepositoryLabel, ForgeError>;
    async fn list_labels_for_repo(&self, repo_id: &str)
        -> Result<Vec<RepositoryLabel>, ForgeError>;
    async fn delete_label(&self, id: &str) -> Result<(), ForgeError>;

    // -------------------------------------------------------------------------
    // Issues & Issue Comments (Batch 09)
    // -------------------------------------------------------------------------
    async fn create_issue(&self, issue: Issue) -> Result<Issue, ForgeError>;
    async fn get_issue_by_number(
        &self,
        repo_id: &str,
        issue_number: i64,
    ) -> Result<Option<Issue>, ForgeError>;
    async fn list_issues_for_repo(
        &self,
        repo_id: &str,
        status: Option<&str>,
    ) -> Result<Vec<Issue>, ForgeError>;
    async fn update_issue(&self, issue: Issue) -> Result<Issue, ForgeError>;

    async fn assign_issue(&self, issue_id: &str, user_id: &str) -> Result<(), ForgeError>;
    async fn unassign_issue(&self, issue_id: &str, user_id: &str) -> Result<(), ForgeError>;
    async fn list_issue_assignees(&self, issue_id: &str) -> Result<Vec<String>, ForgeError>;

    async fn add_issue_label(&self, issue_id: &str, label_id: &str) -> Result<(), ForgeError>;
    async fn remove_issue_label(&self, issue_id: &str, label_id: &str) -> Result<(), ForgeError>;
    async fn list_issue_labels(&self, issue_id: &str) -> Result<Vec<String>, ForgeError>;

    async fn create_issue_comment(&self, comment: IssueComment)
        -> Result<IssueComment, ForgeError>;
    async fn list_issue_comments(&self, issue_id: &str) -> Result<Vec<IssueComment>, ForgeError>;

    // -------------------------------------------------------------------------
    // Pull Requests, Reviews & Inline Comments (Batch 09)
    // -------------------------------------------------------------------------
    async fn create_pull_request(&self, pr: PullRequest) -> Result<PullRequest, ForgeError>;
    async fn get_pull_request_by_number(
        &self,
        repo_id: &str,
        pr_number: i64,
    ) -> Result<Option<PullRequest>, ForgeError>;
    async fn list_pull_requests_for_repo(
        &self,
        repo_id: &str,
        state: Option<&str>,
    ) -> Result<Vec<PullRequest>, ForgeError>;
    async fn update_pull_request(&self, pr: PullRequest) -> Result<PullRequest, ForgeError>;

    async fn request_pr_reviewer(
        &self,
        pr_id: &str,
        reviewer_id: &str,
        requested_by: &str,
    ) -> Result<(), ForgeError>;
    async fn list_pr_requested_reviewers(&self, pr_id: &str) -> Result<Vec<String>, ForgeError>;

    async fn create_pr_review(
        &self,
        review: PullRequestReview,
    ) -> Result<PullRequestReview, ForgeError>;
    async fn list_pr_reviews(&self, pr_id: &str) -> Result<Vec<PullRequestReview>, ForgeError>;
    async fn dismiss_pr_review(
        &self,
        review_id: &str,
        dismissed_by: &str,
        now_nanos: i64,
    ) -> Result<(), ForgeError>;

    async fn create_pr_review_comment(
        &self,
        comment: PullRequestReviewComment,
    ) -> Result<PullRequestReviewComment, ForgeError>;
    async fn list_pr_review_comments(
        &self,
        pr_id: &str,
    ) -> Result<Vec<PullRequestReviewComment>, ForgeError>;
    async fn resolve_pr_review_comment(
        &self,
        comment_id: &str,
        resolved: bool,
    ) -> Result<(), ForgeError>;

    // -------------------------------------------------------------------------
    // Branch Protection Rules (Batch 09)
    // -------------------------------------------------------------------------
    async fn set_branch_protection(
        &self,
        rule: BranchProtectionRule,
    ) -> Result<BranchProtectionRule, ForgeError>;
    async fn get_branch_protection(
        &self,
        repo_id: &str,
        branch: &str,
    ) -> Result<Option<BranchProtectionRule>, ForgeError>;
    async fn list_branch_protection_rules(
        &self,
        repo_id: &str,
    ) -> Result<Vec<BranchProtectionRule>, ForgeError>;
    async fn delete_branch_protection(&self, rule_id: &str) -> Result<(), ForgeError>;

    // -------------------------------------------------------------------------
    // Notifications & Activity Events (Batch 09)
    // -------------------------------------------------------------------------
    async fn create_notification(
        &self,
        notif: UserNotification,
    ) -> Result<UserNotification, ForgeError>;
    async fn list_user_notifications(
        &self,
        user_id: &str,
        unread_only: bool,
    ) -> Result<Vec<UserNotification>, ForgeError>;
    async fn mark_notification_read(&self, notif_id: &str, user_id: &str)
        -> Result<(), ForgeError>;
    async fn mark_all_notifications_read(&self, user_id: &str) -> Result<(), ForgeError>;

    async fn record_activity_event(&self, event: RepositoryActivityEvent)
        -> Result<(), ForgeError>;
    async fn list_activity_events_for_repo(
        &self,
        repo_id: &str,
        limit: usize,
    ) -> Result<Vec<RepositoryActivityEvent>, ForgeError>;

    async fn record_audit_event(&self, event: AuditEvent) -> Result<(), ForgeError>;

    // -------------------------------------------------------------------------
    // Batch 11: Artifacts, Releases & Packages
    // -------------------------------------------------------------------------
    async fn get_or_create_quota(
        &self,
        owner_type: &str,
        owner_id: &str,
    ) -> Result<ArtifactQuota, ForgeError>;
    async fn update_quota_usage(
        &self,
        owner_id: &str,
        delta_logical: i64,
        delta_physical: i64,
    ) -> Result<(), ForgeError>;

    async fn create_upload_session(
        &self,
        session: ArtifactUploadSession,
    ) -> Result<ArtifactUploadSession, ForgeError>;
    async fn get_upload_session(
        &self,
        session_id: &str,
    ) -> Result<Option<ArtifactUploadSession>, ForgeError>;
    async fn update_upload_session_state(
        &self,
        session_id: &str,
        state: &str,
    ) -> Result<(), ForgeError>;

    async fn save_artifact_manifest(
        &self,
        manifest: StoredArtifactManifest,
    ) -> Result<(), ForgeError>;
    async fn get_artifact_manifest(
        &self,
        manifest_id: &str,
    ) -> Result<Option<StoredArtifactManifest>, ForgeError>;

    async fn create_release(&self, release: Release) -> Result<Release, ForgeError>;
    async fn get_release_by_id(&self, id: &str) -> Result<Option<Release>, ForgeError>;
    async fn get_release_by_tag(
        &self,
        repo_id: &str,
        tag_name: &str,
    ) -> Result<Option<Release>, ForgeError>;
    async fn list_releases_for_repo(
        &self,
        repo_id: &str,
        include_drafts: bool,
    ) -> Result<Vec<Release>, ForgeError>;
    async fn update_release(&self, release: Release) -> Result<Release, ForgeError>;
    async fn delete_release(&self, id: &str) -> Result<(), ForgeError>;

    async fn create_release_asset(&self, asset: ReleaseAsset) -> Result<ReleaseAsset, ForgeError>;
    async fn get_release_asset(&self, id: &str) -> Result<Option<ReleaseAsset>, ForgeError>;
    async fn list_assets_for_release(
        &self,
        release_id: &str,
    ) -> Result<Vec<ReleaseAsset>, ForgeError>;
    async fn delete_release_asset(&self, id: &str) -> Result<(), ForgeError>;
    async fn increment_asset_downloads(&self, id: &str) -> Result<(), ForgeError>;

    async fn create_artifact_namespace(
        &self,
        ns: ArtifactNamespace,
    ) -> Result<ArtifactNamespace, ForgeError>;
    async fn get_artifact_namespace_by_name(
        &self,
        name: &str,
    ) -> Result<Option<ArtifactNamespace>, ForgeError>;
    async fn list_artifact_namespaces(&self) -> Result<Vec<ArtifactNamespace>, ForgeError>;
    async fn create_artifact_version(
        &self,
        ver: ArtifactVersion,
    ) -> Result<ArtifactVersion, ForgeError>;
    async fn get_artifact_version(
        &self,
        namespace_id: &str,
        artifact_name: &str,
        version: &str,
    ) -> Result<Option<ArtifactVersion>, ForgeError>;
    async fn list_artifact_versions(
        &self,
        namespace_id: &str,
        artifact_name: &str,
    ) -> Result<Vec<ArtifactVersion>, ForgeError>;

    async fn create_package(&self, pkg: Package) -> Result<Package, ForgeError>;
    async fn get_package_by_name(
        &self,
        ecosystem: &str,
        name: &str,
    ) -> Result<Option<Package>, ForgeError>;
    async fn list_packages(&self, ecosystem: &str) -> Result<Vec<Package>, ForgeError>;
    async fn create_package_version(
        &self,
        ver: PackageVersion,
    ) -> Result<PackageVersion, ForgeError>;
    async fn get_package_version(
        &self,
        package_id: &str,
        version: &str,
    ) -> Result<Option<PackageVersion>, ForgeError>;
    async fn list_package_versions(
        &self,
        package_id: &str,
    ) -> Result<Vec<PackageVersion>, ForgeError>;
    async fn set_package_version_yanked(
        &self,
        package_id: &str,
        version: &str,
        yanked: bool,
    ) -> Result<(), ForgeError>;

    // -------------------------------------------------------------------------
    // Webhooks (Batch 12)
    // -------------------------------------------------------------------------
    async fn create_webhook_subscription(
        &self,
        sub: WebhookSubscription,
    ) -> Result<WebhookSubscription, ForgeError>;
    async fn get_webhook_subscription(
        &self,
        id: &str,
    ) -> Result<Option<WebhookSubscription>, ForgeError>;
    async fn list_webhook_subscriptions(
        &self,
        owner_type: &str,
        owner_id: &str,
    ) -> Result<Vec<WebhookSubscription>, ForgeError>;
    async fn update_webhook_subscription(
        &self,
        sub: WebhookSubscription,
    ) -> Result<WebhookSubscription, ForgeError>;
    async fn delete_webhook_subscription(&self, id: &str) -> Result<(), ForgeError>;

    async fn create_webhook_delivery(
        &self,
        delivery: WebhookDelivery,
    ) -> Result<WebhookDelivery, ForgeError>;
    async fn list_webhook_deliveries(
        &self,
        subscription_id: &str,
        limit: i64,
    ) -> Result<Vec<WebhookDelivery>, ForgeError>;

    async fn enqueue_webhook_outbox(
        &self,
        item: WebhookOutboxItem,
    ) -> Result<WebhookOutboxItem, ForgeError>;
    async fn count_pending_webhook_outbox(&self, now_nanos: i64) -> Result<usize, ForgeError>;
    async fn claim_pending_webhook_outbox(
        &self,
        limit: i64,
        now_nanos: i64,
    ) -> Result<Vec<WebhookOutboxItem>, ForgeError>;
    async fn update_webhook_outbox_status(
        &self,
        id: &str,
        status: &str,
        error: Option<&str>,
        next_attempt_nanos: i64,
    ) -> Result<(), ForgeError>;

    // -------------------------------------------------------------------------
    // Workflows, Runs, Jobs, Steps, Logs (Batch 12)
    // -------------------------------------------------------------------------
    async fn upsert_workflow_definition(
        &self,
        def: WorkflowDefinition,
    ) -> Result<WorkflowDefinition, ForgeError>;
    async fn get_workflow_definition(
        &self,
        repo_id: &str,
        path: &str,
    ) -> Result<Option<WorkflowDefinition>, ForgeError>;
    async fn list_workflow_definitions(
        &self,
        repo_id: &str,
    ) -> Result<Vec<WorkflowDefinition>, ForgeError>;

    async fn create_workflow_run(&self, run: WorkflowRun) -> Result<WorkflowRun, ForgeError>;
    async fn get_workflow_run(&self, id: &str) -> Result<Option<WorkflowRun>, ForgeError>;
    async fn list_workflow_runs(
        &self,
        repo_id: &str,
        limit: i64,
    ) -> Result<Vec<WorkflowRun>, ForgeError>;
    async fn list_active_workflow_runs_for_repo(
        &self,
        repo_id: &str,
    ) -> Result<Vec<WorkflowRun>, ForgeError>;
    async fn list_active_workflow_repo_ids(&self) -> Result<Vec<String>, ForgeError>;
    async fn update_workflow_run_status(
        &self,
        id: &str,
        status: &str,
        conclusion: Option<&str>,
        started_at: Option<i64>,
        completed_at: Option<i64>,
    ) -> Result<(), ForgeError>;

    async fn create_workflow_job(&self, job: WorkflowJob) -> Result<WorkflowJob, ForgeError>;
    async fn get_workflow_job(&self, id: &str) -> Result<Option<WorkflowJob>, ForgeError>;
    async fn list_workflow_jobs(&self, run_id: &str) -> Result<Vec<WorkflowJob>, ForgeError>;
    async fn list_queued_workflow_jobs(
        &self,
        now_nanos: i64,
    ) -> Result<Vec<WorkflowJob>, ForgeError>;
    async fn assign_workflow_job(
        &self,
        job_id: &str,
        runner_id: &str,
        fencing_token: i64,
        lease_expires_at: i64,
    ) -> Result<(), ForgeError>;
    async fn update_workflow_job_status(
        &self,
        job_id: &str,
        status: &str,
        exit_code: Option<i32>,
        started_at: Option<i64>,
        completed_at: Option<i64>,
    ) -> Result<(), ForgeError>;

    async fn create_workflow_step(&self, step: WorkflowStep) -> Result<WorkflowStep, ForgeError>;
    async fn list_workflow_steps(&self, job_id: &str) -> Result<Vec<WorkflowStep>, ForgeError>;
    async fn update_workflow_step(
        &self,
        step_id: &str,
        status: &str,
        exit_code: Option<i32>,
        started_at: Option<i64>,
        completed_at: Option<i64>,
    ) -> Result<(), ForgeError>;

    async fn append_workflow_log(&self, log: WorkflowLogChunk) -> Result<(), ForgeError>;
    async fn list_workflow_logs(&self, job_id: &str) -> Result<Vec<WorkflowLogChunk>, ForgeError>;

    // -------------------------------------------------------------------------
    // Runners (Batch 12)
    // -------------------------------------------------------------------------
    async fn register_runner(
        &self,
        runner: RunnerRegistration,
    ) -> Result<RunnerRegistration, ForgeError>;
    async fn get_runner_by_id(&self, id: &str) -> Result<Option<RunnerRegistration>, ForgeError>;
    async fn get_runner_by_token_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<RunnerRegistration>, ForgeError>;
    async fn list_runners(&self) -> Result<Vec<RunnerRegistration>, ForgeError>;
    async fn update_runner_heartbeat(
        &self,
        runner_id: &str,
        active_jobs: i32,
        now_nanos: i64,
    ) -> Result<(), ForgeError>;
    async fn update_runner_status(&self, runner_id: &str, status: &str) -> Result<(), ForgeError>;

    // -------------------------------------------------------------------------
    // Workflow Secrets & PR Check Results (Batch 12)
    // -------------------------------------------------------------------------
    async fn upsert_workflow_secret(
        &self,
        secret: WorkflowSecret,
    ) -> Result<WorkflowSecret, ForgeError>;
    async fn get_workflow_secret(
        &self,
        owner_type: &str,
        owner_id: &str,
        name: &str,
    ) -> Result<Option<WorkflowSecret>, ForgeError>;
    async fn list_workflow_secrets(
        &self,
        owner_type: &str,
        owner_id: &str,
    ) -> Result<Vec<WorkflowSecret>, ForgeError>;
    async fn delete_workflow_secret(
        &self,
        owner_type: &str,
        owner_id: &str,
        name: &str,
    ) -> Result<(), ForgeError>;

    async fn upsert_workflow_check_result(
        &self,
        check: WorkflowCheckResult,
    ) -> Result<WorkflowCheckResult, ForgeError>;
    async fn get_workflow_check_result(
        &self,
        repo_id: &str,
        commit_oid: &str,
        check_name: &str,
    ) -> Result<Option<WorkflowCheckResult>, ForgeError>;
    async fn list_workflow_check_results(
        &self,
        repo_id: &str,
        commit_oid: &str,
    ) -> Result<Vec<WorkflowCheckResult>, ForgeError>;

    // -------------------------------------------------------------------------
    // Discussions (Batch 13)
    // -------------------------------------------------------------------------
    async fn create_discussion_category(
        &self,
        cat: DiscussionCategory,
    ) -> Result<DiscussionCategory, ForgeError>;
    async fn get_discussion_category(
        &self,
        id: &str,
    ) -> Result<Option<DiscussionCategory>, ForgeError>;
    async fn list_discussion_categories(
        &self,
        repo_id: &str,
    ) -> Result<Vec<DiscussionCategory>, ForgeError>;

    async fn create_discussion(&self, disc: Discussion) -> Result<Discussion, ForgeError>;
    async fn get_discussion(&self, id: &str) -> Result<Option<Discussion>, ForgeError>;
    async fn get_discussion_by_number(
        &self,
        repo_id: &str,
        number: i32,
    ) -> Result<Option<Discussion>, ForgeError>;
    async fn list_discussions(
        &self,
        repo_id: &str,
        category_id: Option<&str>,
    ) -> Result<Vec<Discussion>, ForgeError>;
    async fn update_discussion(&self, disc: Discussion) -> Result<Discussion, ForgeError>;
    async fn delete_discussion(&self, id: &str) -> Result<(), ForgeError>;

    async fn create_discussion_comment(
        &self,
        comment: DiscussionComment,
    ) -> Result<DiscussionComment, ForgeError>;
    async fn get_discussion_comment(
        &self,
        id: &str,
    ) -> Result<Option<DiscussionComment>, ForgeError>;
    async fn list_discussion_comments(
        &self,
        discussion_id: &str,
    ) -> Result<Vec<DiscussionComment>, ForgeError>;
    async fn update_discussion_comment(
        &self,
        comment: DiscussionComment,
    ) -> Result<DiscussionComment, ForgeError>;
    async fn delete_discussion_comment(&self, id: &str) -> Result<(), ForgeError>;

    async fn add_discussion_reaction(
        &self,
        reaction: DiscussionReaction,
    ) -> Result<DiscussionReaction, ForgeError>;
    async fn remove_discussion_reaction(
        &self,
        subject_type: &str,
        subject_id: &str,
        user_id: &str,
        reaction: &str,
    ) -> Result<(), ForgeError>;
    async fn list_discussion_reactions(
        &self,
        subject_type: &str,
        subject_id: &str,
    ) -> Result<Vec<DiscussionReaction>, ForgeError>;

    async fn set_discussion_subscription(
        &self,
        sub: DiscussionSubscription,
    ) -> Result<DiscussionSubscription, ForgeError>;
    async fn get_discussion_subscription(
        &self,
        user_id: &str,
        discussion_id: &str,
    ) -> Result<Option<DiscussionSubscription>, ForgeError>;

    // -------------------------------------------------------------------------
    // Discovery, Topics, Languages, Profiles (Batch 13)
    // -------------------------------------------------------------------------
    async fn set_repository_topics(
        &self,
        repo_id: &str,
        topics: &[String],
    ) -> Result<Vec<RepositoryTopic>, ForgeError>;
    async fn list_repository_topics(
        &self,
        repo_id: &str,
    ) -> Result<Vec<RepositoryTopic>, ForgeError>;
    async fn list_popular_topics(&self, limit: usize) -> Result<Vec<PopularTopicDto>, ForgeError>;
    async fn list_repositories_by_topic(
        &self,
        topic: &str,
    ) -> Result<Vec<ForgeRepository>, ForgeError>;

    async fn set_repository_languages(
        &self,
        repo_id: &str,
        langs: &[RepositoryLanguage],
    ) -> Result<Vec<RepositoryLanguage>, ForgeError>;
    async fn list_repository_languages(
        &self,
        repo_id: &str,
    ) -> Result<Vec<RepositoryLanguage>, ForgeError>;

    async fn get_extended_user_profile(
        &self,
        user_id: &str,
    ) -> Result<Option<ExtendedUserProfile>, ForgeError>;
    async fn upsert_extended_user_profile(
        &self,
        profile: ExtendedUserProfile,
    ) -> Result<ExtendedUserProfile, ForgeError>;

    // -------------------------------------------------------------------------
    // Security Center (Batch 13)
    // -------------------------------------------------------------------------
    async fn create_security_scan(&self, scan: SecurityScan) -> Result<SecurityScan, ForgeError>;
    async fn list_security_scans(&self, repo_id: &str) -> Result<Vec<SecurityScan>, ForgeError>;

    async fn create_secret_scan_finding(
        &self,
        finding: SecretScanFinding,
    ) -> Result<SecretScanFinding, ForgeError>;
    async fn list_secret_scan_findings(
        &self,
        repo_id: &str,
        status_filter: Option<&str>,
    ) -> Result<Vec<SecretScanFinding>, ForgeError>;
    async fn update_secret_scan_finding_status(
        &self,
        id: &str,
        status: &str,
        now_nanos: i64,
    ) -> Result<(), ForgeError>;

    async fn upsert_dependency_inventory(
        &self,
        items: &[DependencyInventory],
    ) -> Result<(), ForgeError>;
    async fn list_dependency_inventory(
        &self,
        repo_id: &str,
    ) -> Result<Vec<DependencyInventory>, ForgeError>;

    async fn upsert_security_advisory(
        &self,
        advisory: SecurityAdvisory,
    ) -> Result<SecurityAdvisory, ForgeError>;
    async fn list_security_advisories(
        &self,
        ecosystem: &str,
    ) -> Result<Vec<SecurityAdvisory>, ForgeError>;

    async fn create_dependency_vulnerability(
        &self,
        vuln: DependencyVulnerability,
    ) -> Result<DependencyVulnerability, ForgeError>;
    async fn list_dependency_vulnerabilities(
        &self,
        repo_id: &str,
        status_filter: Option<&str>,
    ) -> Result<Vec<DependencyVulnerability>, ForgeError>;
    async fn update_dependency_vulnerability_status(
        &self,
        id: &str,
        status: &str,
        now_nanos: i64,
    ) -> Result<(), ForgeError>;

    // -------------------------------------------------------------------------
    // SSH Keys & Deploy Keys (Batch 18)
    // -------------------------------------------------------------------------
    async fn create_user_ssh_key(&self, key: UserSshKey) -> Result<UserSshKey, ForgeError>;
    async fn get_user_ssh_key_by_id(&self, id: &str) -> Result<Option<UserSshKey>, ForgeError>;
    async fn get_user_ssh_key_by_fingerprint(
        &self,
        fingerprint: &str,
    ) -> Result<Option<UserSshKey>, ForgeError>;
    async fn list_user_ssh_keys(&self, user_id: &str) -> Result<Vec<UserSshKey>, ForgeError>;
    async fn update_user_ssh_key_last_used(
        &self,
        id: &str,
        now_nanos: i64,
    ) -> Result<(), ForgeError>;
    async fn revoke_user_ssh_key(&self, id: &str, user_id: &str) -> Result<(), ForgeError>;

    async fn create_deploy_key(&self, key: DeployKey) -> Result<DeployKey, ForgeError>;
    async fn get_deploy_key_by_id(&self, id: &str) -> Result<Option<DeployKey>, ForgeError>;
    async fn get_deploy_key_by_fingerprint(
        &self,
        fingerprint: &str,
    ) -> Result<Option<DeployKey>, ForgeError>;
    async fn list_deploy_keys(&self, repo_id: &str) -> Result<Vec<DeployKey>, ForgeError>;
    async fn update_deploy_key_last_used(&self, id: &str, now_nanos: i64)
        -> Result<(), ForgeError>;
    async fn revoke_deploy_key(&self, id: &str, repo_id: &str) -> Result<(), ForgeError>;

    // -------------------------------------------------------------------------
    // Git LFS File Locks (Batch 18)
    // -------------------------------------------------------------------------
    async fn create_lfs_lock(&self, lock: LfsLock) -> Result<LfsLock, ForgeError>;
    async fn get_lfs_lock_by_id(&self, id: &str) -> Result<Option<LfsLock>, ForgeError>;
    async fn get_lfs_lock_by_path(
        &self,
        repo_id: &str,
        path: &str,
    ) -> Result<Option<LfsLock>, ForgeError>;
    async fn list_lfs_locks(
        &self,
        repo_id: &str,
        path: Option<&str>,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> Result<(Vec<LfsLock>, Option<String>), ForgeError>;
    async fn delete_lfs_lock(&self, id: &str) -> Result<Option<LfsLock>, ForgeError>;
    async fn create_lfs_lock_audit(&self, audit: LfsLockAudit) -> Result<(), ForgeError>;
}
