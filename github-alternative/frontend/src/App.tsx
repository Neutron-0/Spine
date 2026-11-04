import React, { useEffect, useState } from 'react';
import {
  RepositoryDto,
  CommitDto,
  BranchDto,
  TreeEntryDto,
  BlobContentDto,
  UserDto,
  SessionResponse,
  IssueDto,
  IssueCommentDto,
  PullRequestDto,
  PullRequestReviewDto,
  PullRequestReviewCommentDto,
  PullRequestDiffDto,
  MergeabilityReport,
  OrganizationDto,
  OrganizationMemberDto,
  TeamDto,
  BranchProtectionRule,
  NotificationDto,
  ReleaseDto,
  PackageDto,
  ArtifactQuota,
  WebhookSubscriptionDto,
  WebhookDeliveryDto,
  WorkflowDefinitionDto,
  WorkflowRunDto,
  WorkflowJobDto,
  WorkflowStepDto,
  WorkflowLogChunkDto,
  RunnerRegistrationDto,
  SecretSummaryDto,
} from './types';

export const App: React.FC = () => {
  // Authentication & Session
  const [currentUser, setCurrentUser] = useState<UserDto | null>(null);
  const [authToken, setAuthToken] = useState<string>(() => localStorage.getItem('spine_token') || '');
  const [showAuthModal, setShowAuthModal] = useState<boolean>(false);
  const [authMode, setAuthMode] = useState<'login' | 'register'>('login');
  const [authForm, setAuthForm] = useState({ username: '', email: '', password: '' });
  const [authError, setAuthError] = useState<string | null>(null);

  // Repositories & Active View
  const [repos, setRepos] = useState<RepositoryDto[]>([]);
  const [selectedRepo, setSelectedRepo] = useState<RepositoryDto | null>(null);
  const [branches, setBranches] = useState<BranchDto[]>([]);
  const [currentBranch, setCurrentBranch] = useState<string>('main');
  const [commits, setCommits] = useState<CommitDto[]>([]);
  const [activeTab, setActiveTab] = useState<'code' | 'issues' | 'pulls' | 'orgs' | 'protection' | 'releases' | 'packages' | 'actions' | 'webhooks' | 'runners' | 'secrets'>('code');
  const [codeSubTab, setCodeSubTab] = useState<'files' | 'commits'>('files');
  const [treeEntries, setTreeEntries] = useState<TreeEntryDto[]>([]);
  const [selectedBlob, setSelectedBlob] = useState<BlobContentDto | null>(null);
  const [currentPath, setCurrentPath] = useState<string>('');
  const [loading, setLoading] = useState<boolean>(true);

  // Issues State
  const [issues, setIssues] = useState<IssueDto[]>([]);
  const [issueFilter, setIssueFilter] = useState<'OPEN' | 'CLOSED'>('OPEN');
  const [selectedIssue, setSelectedIssue] = useState<IssueDto | null>(null);
  const [issueComments, setIssueComments] = useState<IssueCommentDto[]>([]);
  const [newCommentBody, setNewCommentBody] = useState<string>('');
  const [showNewIssueModal, setShowNewIssueModal] = useState<boolean>(false);
  const [newIssueForm, setNewIssueForm] = useState({ title: '', body: '' });

  // Pull Requests State
  const [pullRequests, setPullRequests] = useState<PullRequestDto[]>([]);
  const [prFilter, setPrFilter] = useState<'OPEN' | 'CLOSED' | 'MERGED'>('OPEN');
  const [selectedPr, setSelectedPr] = useState<PullRequestDto | null>(null);
  const [prSubTab, setPrSubTab] = useState<'conversation' | 'diff'>('conversation');
  const [prDiff, setPrDiff] = useState<PullRequestDiffDto | null>(null);
  const [prMergeability, setPrMergeability] = useState<MergeabilityReport | null>(null);
  const [prReviews, setPrReviews] = useState<PullRequestReviewDto[]>([]);
  const [prComments, setPrComments] = useState<PullRequestReviewCommentDto[]>([]);
  const [showNewPrModal, setShowNewPrModal] = useState<boolean>(false);
  const [newPrForm, setNewPrForm] = useState({ title: '', body: '', base_branch: 'main', head_branch: '' });
  const [reviewState, setReviewState] = useState<'APPROVED' | 'CHANGES_REQUESTED' | 'COMMENTED'>('APPROVED');
  const [reviewBody, setReviewBody] = useState<string>('');
  const [inlineCommentForm, setInlineCommentForm] = useState<{ path: string; line: number; body: string } | null>(null);
  const [mergeStatusMsg, setMergeStatusMsg] = useState<string | null>(null);

  // Releases State (Batch 11)
  const [releases, setReleases] = useState<ReleaseDto[]>([]);
  const [showNewReleaseModal, setShowNewReleaseModal] = useState<boolean>(false);
  const [newReleaseForm, setNewReleaseForm] = useState({ tag_name: '', title: '', body: '', is_draft: false, is_prerelease: false });

  // Packages & Quota State (Batch 11)
  const [packages, setPackages] = useState<PackageDto[]>([]);
  const [artifactQuota, setArtifactQuota] = useState<ArtifactQuota | null>(null);

  // Organizations & Teams
  const [orgs, setOrgs] = useState<OrganizationDto[]>([]);
  const [selectedOrg, setSelectedOrg] = useState<OrganizationDto | null>(null);
  const [orgMembers, setOrgMembers] = useState<OrganizationMemberDto[]>([]);
  const [orgTeams, setOrgTeams] = useState<TeamDto[]>([]);
  const [showNewOrgModal, setShowNewOrgModal] = useState<boolean>(false);
  const [newOrgForm, setNewOrgForm] = useState({ name: '', display_name: '', description: '' });
  const [showNewTeamModal, setShowNewTeamModal] = useState<boolean>(false);
  const [newTeamForm, setNewTeamForm] = useState({ name: '', description: '' });

  // Branch Protection State
  const [protectionRules, setProtectionRules] = useState<BranchProtectionRule[]>([]);
  const [showNewRuleModal, setShowNewRuleModal] = useState<boolean>(false);
  const [newRuleForm, setNewRuleForm] = useState({
    branch_pattern: 'main',
    required_approvals: 1,
    prevent_self_approval: true,
    require_changes_addressed: true,
  });

  // Notifications State
  const [notifications, setNotifications] = useState<NotificationDto[]>([]);
  const [showNotificationsModal, setShowNotificationsModal] = useState<boolean>(false);

  // New Repository Modal
  const [showNewRepoModal, setShowNewRepoModal] = useState<boolean>(false);
  const [newRepoForm, setNewRepoForm] = useState({
    name: '',
    description: '',
    is_private: false,
    durability: 'ThreeCopies',
  });

  const getAuthHeaders = (): Record<string, string> => {
    const headers: Record<string, string> = {};
    if (authToken) {
      headers['Authorization'] = `Bearer ${authToken}`;
    }
    return headers;
  };

  // Check active session on mount
  useEffect(() => {
    fetch('/api/v1/auth/session', { headers: getAuthHeaders() })
      .then((res) => (res.ok ? res.json() : null))
      .then((data: SessionResponse | null) => {
        if (data && data.authenticated && data.user) {
          setCurrentUser(data.user);
        } else {
          setCurrentUser(null);
        }
      })
      .catch(() => setCurrentUser(null));
  }, [authToken]);

  // Fetch repositories
  const fetchRepos = () => {
    setLoading(true);
    fetch('/api/v1/repos', { headers: getAuthHeaders() })
      .then((res) => (res.ok ? res.json() : []))
      .then((data: RepositoryDto[]) => {
        setRepos(data);
        if (data.length > 0 && !selectedRepo) {
          setSelectedRepo(data[0]);
        }
        setLoading(false);
      })
      .catch(() => setLoading(false));
  };

  useEffect(() => {
    fetchRepos();
  }, [currentUser]);

  // Fetch notifications
  const fetchNotifications = () => {
    if (!currentUser) return;
    fetch('/api/v1/notifications?unread_only=true', { headers: getAuthHeaders() })
      .then((res) => (res.ok ? res.json() : []))
      .then((data: NotificationDto[]) => setNotifications(data))
      .catch(() => {});
  };

  useEffect(() => {
    fetchNotifications();
    const interval = setInterval(fetchNotifications, 15000);
    return () => clearInterval(interval);
  }, [currentUser]);

  // Fetch organizations
  const fetchOrgs = () => {
    if (!currentUser) return;
    fetch('/api/v1/orgs', { headers: getAuthHeaders() })
      .then((res) => (res.ok ? res.json() : []))
      .then((data: OrganizationDto[]) => {
        setOrgs(data);
        if (data.length > 0 && !selectedOrg) {
          setSelectedOrg(data[0]);
        }
      })
      .catch(() => {});
  };

  useEffect(() => {
    fetchOrgs();
  }, [currentUser]);

  useEffect(() => {
    if (!selectedOrg) {
      setOrgMembers([]);
      setOrgTeams([]);
      return;
    }
    fetch(`/api/v1/orgs/${selectedOrg.name}/members`, { headers: getAuthHeaders() })
      .then((res) => (res.ok ? res.json() : []))
      .then((data: OrganizationMemberDto[]) => setOrgMembers(data))
      .catch(() => setOrgMembers([]));

    fetch(`/api/v1/orgs/${selectedOrg.name}/teams`, { headers: getAuthHeaders() })
      .then((res) => (res.ok ? res.json() : []))
      .then((data: TeamDto[]) => setOrgTeams(data))
      .catch(() => setOrgTeams([]));
  }, [selectedOrg]);

  // Load branches, commits, tree, issues, PRs, and rules when repository changes
  useEffect(() => {
    if (!selectedRepo) return;
    setCurrentBranch(selectedRepo.default_branch || 'main');
    setSelectedBlob(null);
    setCurrentPath('');
    setSelectedIssue(null);
    setSelectedPr(null);

    // 1. Branches
    fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/branches`, { headers: getAuthHeaders() })
      .then((res) => (res.ok ? res.json() : []))
      .then((data: BranchDto[]) => setBranches(data))
      .catch(() => setBranches([{ name: 'main', commit_id: '4f2910...' }]));

    // 2. Commits
    fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/commits?ref=${selectedRepo.default_branch || 'main'}`, {
      headers: getAuthHeaders(),
    })
      .then((res) => (res.ok ? res.json() : []))
      .then((data: CommitDto[]) => setCommits(data))
      .catch(() => setCommits([]));

    // 3. Root tree
    fetchTree(selectedRepo.owner, selectedRepo.name, selectedRepo.default_branch || 'main', '');

    // 4. Issues
    fetchIssues(selectedRepo.owner, selectedRepo.name);

    // 5. Pull Requests
    fetchPullRequests(selectedRepo.owner, selectedRepo.name);

    // 6. Branch Protection Rules
    fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/protection`, { headers: getAuthHeaders() })
      .then((res) => (res.ok ? res.json() : []))
      .then((rules: BranchProtectionRule[]) => setProtectionRules(rules))
      .catch(() => {});

    // 7. Releases (Batch 11)
    fetchReleases(selectedRepo.owner, selectedRepo.name);
  }, [selectedRepo]);

  const fetchReleases = (owner: string, name: string) => {
    fetch(`/api/v1/repos/${owner}/${name}/releases?include_drafts=true`, { headers: getAuthHeaders() })
      .then((res) => (res.ok ? res.json() : []))
      .then((data: ReleaseDto[]) => setReleases(Array.isArray(data) ? data : []))
      .catch(() => setReleases([]));
  };

  const fetchPackages = () => {
    fetch('/api/v1/packages', { headers: getAuthHeaders() })
      .then((res) => (res.ok ? res.json() : []))
      .then((data: PackageDto[]) => setPackages(Array.isArray(data) ? data : []))
      .catch(() => setPackages([]));

    if (authToken) {
      fetch('/api/v1/artifacts/quota', { headers: getAuthHeaders() })
        .then((res) => (res.ok ? res.json() : null))
        .then((data: ArtifactQuota | null) => setArtifactQuota(data))
        .catch(() => {});
    }
  };

  useEffect(() => {
    if (activeTab === 'packages') {
      fetchPackages();
    } else if (activeTab === 'actions') {
      fetchWorkflowsAndRuns();
    } else if (activeTab === 'webhooks') {
      fetchWebhooks();
    } else if (activeTab === 'runners') {
      fetchRunners();
    } else if (activeTab === 'secrets') {
      fetchSecrets();
    }
  }, [activeTab, selectedRepo]);

  // -------------------------------------------------------------
  // Batch 12: Workflows, Actions, Webhooks, Runners & Secrets State
  // -------------------------------------------------------------
  const [workflows, setWorkflows] = useState<WorkflowDefinitionDto[]>([]);
  const [workflowRuns, setWorkflowRuns] = useState<WorkflowRunDto[]>([]);
  const [selectedRun, setSelectedRun] = useState<WorkflowRunDto | null>(null);
  const [runJobs, setRunJobs] = useState<WorkflowJobDto[]>([]);
  const [selectedJob, setSelectedJob] = useState<WorkflowJobDto | null>(null);
  const [jobSteps, setJobSteps] = useState<WorkflowStepDto[]>([]);
  const [jobLogs, setJobLogs] = useState<WorkflowLogChunkDto[]>([]);

  const [webhooks, setWebhooks] = useState<WebhookSubscriptionDto[]>([]);
  const [newWebhookUrl, setNewWebhookUrl] = useState<string>('');
  const [newWebhookSecret, setNewWebhookSecret] = useState<string>('');
  const [newWebhookEvents, setNewWebhookEvents] = useState<string>('push,pull_request');
  const [selectedWebhook, setSelectedWebhook] = useState<WebhookSubscriptionDto | null>(null);
  const [webhookDeliveries, setWebhookDeliveries] = useState<WebhookDeliveryDto[]>([]);

  const [runners, setRunners] = useState<RunnerRegistrationDto[]>([]);
  const [newRunnerName, setNewRunnerName] = useState<string>('');
  const [newRunnerLabels, setNewRunnerLabels] = useState<string>('windows,self-hosted');
  const [registeredToken, setRegisteredToken] = useState<string | null>(null);

  const [secrets, setSecrets] = useState<SecretSummaryDto[]>([]);
  const [newSecretName, setNewSecretName] = useState<string>('');
  const [newSecretValue, setNewSecretValue] = useState<string>('');

  const fetchWorkflowsAndRuns = () => {
    if (!selectedRepo) return;
    fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/actions/workflows`, { headers: getAuthHeaders() })
      .then(res => res.ok ? res.json() : [])
      .then(data => setWorkflows(Array.isArray(data) ? data : []))
      .catch(() => setWorkflows([]));

    fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/actions/runs`, { headers: getAuthHeaders() })
      .then(res => res.ok ? res.json() : [])
      .then(data => setWorkflowRuns(Array.isArray(data) ? data : []))
      .catch(() => setWorkflowRuns([]));
  };

  const fetchRunDetails = (runId: string) => {
    fetch(`/api/v1/actions/runs/${runId}`, { headers: getAuthHeaders() })
      .then(res => res.ok ? res.json() : null)
      .then(run => {
        setSelectedRun(run);
        if (run) {
          fetch(`/api/v1/actions/runs/${runId}/jobs`, { headers: getAuthHeaders() })
            .then(res => res.ok ? res.json() : [])
            .then(jobs => {
              setRunJobs(jobs);
              if (jobs.length > 0) {
                selectJobDetails(jobs[0]);
              }
            })
            .catch(() => setRunJobs([]));
        }
      });
  };

  const selectJobDetails = (job: WorkflowJobDto) => {
    setSelectedJob(job);
    fetch(`/api/v1/actions/jobs/${job.id}/steps`, { headers: getAuthHeaders() })
      .then(res => res.ok ? res.json() : [])
      .then(steps => setJobSteps(steps))
      .catch(() => setJobSteps([]));

    fetch(`/api/v1/actions/jobs/${job.id}/logs`, { headers: getAuthHeaders() })
      .then(res => res.ok ? res.json() : [])
      .then(logs => setJobLogs(logs))
      .catch(() => setJobLogs([]));
  };

  const fetchWebhooks = () => {
    if (!selectedRepo) return;
    fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/webhooks`, { headers: getAuthHeaders() })
      .then(res => res.ok ? res.json() : [])
      .then(data => setWebhooks(Array.isArray(data) ? data : []))
      .catch(() => setWebhooks([]));
  };

  const fetchWebhookDeliveries = (subId: string) => {
    fetch(`/api/v1/webhooks/${subId}/deliveries`, { headers: getAuthHeaders() })
      .then(res => res.ok ? res.json() : [])
      .then(data => setWebhookDeliveries(Array.isArray(data) ? data : []))
      .catch(() => setWebhookDeliveries([]));
  };

  const fetchRunners = () => {
    fetch('/api/v1/runners', { headers: getAuthHeaders() })
      .then(res => res.ok ? res.json() : [])
      .then(data => setRunners(Array.isArray(data) ? data : []))
      .catch(() => setRunners([]));
  };

  const fetchSecrets = () => {
    if (!selectedRepo) return;
    fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/secrets`, { headers: getAuthHeaders() })
      .then(res => res.ok ? res.json() : [])
      .then(data => setSecrets(Array.isArray(data) ? data : []))
      .catch(() => setSecrets([]));
  };

  const handleCreateRelease = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!selectedRepo || !newReleaseForm.tag_name.trim() || !newReleaseForm.title.trim()) return;

    try {
      const res = await fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/releases`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          ...getAuthHeaders(),
        },
        body: JSON.stringify({
          tag_name: newReleaseForm.tag_name.trim(),
          target_commit_id: commits[0]?.id || 'main',
          title: newReleaseForm.title.trim(),
          body: newReleaseForm.body,
          is_draft: newReleaseForm.is_draft,
          is_prerelease: newReleaseForm.is_prerelease,
        }),
      });

      if (res.ok) {
        setShowNewReleaseModal(false);
        setNewReleaseForm({ tag_name: '', title: '', body: '', is_draft: false, is_prerelease: false });
        fetchReleases(selectedRepo.owner, selectedRepo.name);
      } else {
        const err = await res.json();
        alert(err.error?.message || 'Failed to create release');
      }
    } catch (err: any) {
      alert(err.message || 'Error creating release');
    }
  };

  const fetchTree = (owner: string, name: string, ref: string, path: string) => {
    const url = `/api/v1/repos/${owner}/${name}/tree/${encodeURIComponent(ref)}${path ? `?path=${encodeURIComponent(path)}` : ''}`;
    fetch(url, { headers: getAuthHeaders() })
      .then((res) => (res.ok ? res.json() : []))
      .then((entries: TreeEntryDto[]) => setTreeEntries(entries))
      .catch(() => setTreeEntries([]));
  };

  const fetchIssues = (owner: string, name: string) => {
    fetch(`/api/v1/repos/${owner}/${name}/issues`, { headers: getAuthHeaders() })
      .then((res) => (res.ok ? res.json() : []))
      .then((data: IssueDto[]) => setIssues(data))
      .catch(() => setIssues([]));
  };

  const fetchPullRequests = (owner: string, name: string) => {
    fetch(`/api/v1/repos/${owner}/${name}/pulls`, { headers: getAuthHeaders() })
      .then((res) => (res.ok ? res.json() : []))
      .then((data: PullRequestDto[]) => setPullRequests(data))
      .catch(() => setPullRequests([]));
  };

  // Load Issue details and comments
  const handleSelectIssue = (iss: IssueDto) => {
    setSelectedIssue(iss);
    if (!selectedRepo) return;
    fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/issues/${iss.issue_number}/comments`, {
      headers: getAuthHeaders(),
    })
      .then((res) => (res.ok ? res.json() : []))
      .then((cmts: IssueCommentDto[]) => setIssueComments(cmts))
      .catch(() => setIssueComments([]));
  };

  // Load PR details, diff, mergeability, reviews, and comments
  const handleSelectPr = (pr: PullRequestDto) => {
    setSelectedPr(pr);
    setPrSubTab('conversation');
    setMergeStatusMsg(null);
    if (!selectedRepo) return;

    // Diff
    fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/pulls/${pr.pr_number}/diff`, {
      headers: getAuthHeaders(),
    })
      .then((res) => (res.ok ? res.json() : null))
      .then((diff: PullRequestDiffDto | null) => setPrDiff(diff))
      .catch(() => setPrDiff(null));

    // Mergeability
    fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/pulls/${pr.pr_number}/mergeability`, {
      headers: getAuthHeaders(),
    })
      .then((res) => (res.ok ? res.json() : null))
      .then((m: MergeabilityReport | null) => setPrMergeability(m))
      .catch(() => setPrMergeability(null));

    // Reviews
    fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/pulls/${pr.pr_number}/reviews`, {
      headers: getAuthHeaders(),
    })
      .then((res) => (res.ok ? res.json() : []))
      .then((revs: PullRequestReviewDto[]) => setPrReviews(revs))
      .catch(() => setPrReviews([]));

    // Inline comments
    fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/pulls/${pr.pr_number}/comments`, {
      headers: getAuthHeaders(),
    })
      .then((res) => (res.ok ? res.json() : []))
      .then((cmts: PullRequestReviewCommentDto[]) => setPrComments(cmts))
      .catch(() => setPrComments([]));
  };

  const handleFileClick = (entry: TreeEntryDto) => {
    if (!selectedRepo) return;
    if (entry.is_dir) {
      setCurrentPath(entry.path);
      fetchTree(selectedRepo.owner, selectedRepo.name, currentBranch, entry.path);
    } else {
      const url = `/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/blob/${encodeURIComponent(currentBranch)}/${entry.path}`;
      fetch(url, { headers: getAuthHeaders() })
        .then((res) => (res.ok ? res.json() : null))
        .then((blob: BlobContentDto | null) => {
          if (blob) setSelectedBlob(blob);
        })
        .catch(() => {});
    }
  };

  // Auth Submit
  const handleAuthSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setAuthError(null);
    const endpoint = authMode === 'login' ? '/api/v1/auth/login' : '/api/v1/auth/register';
    const payload =
      authMode === 'login'
        ? { login: authForm.username, password: authForm.password }
        : { username: authForm.username, email: authForm.email, password: authForm.password };

    try {
      const res = await fetch(endpoint, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(payload),
      });
      const data = await res.json();
      if (!res.ok) throw new Error(data.message || data.error || 'Authentication failed');

      if (data.token) {
        localStorage.setItem('spine_token', data.token);
        setAuthToken(data.token);
      }
      setCurrentUser(data.user);
      setShowAuthModal(false);
      setAuthForm({ username: '', email: '', password: '' });
      fetchRepos();
    } catch (err: any) {
      setAuthError(err.message || 'An error occurred');
    }
  };

  const handleLogout = async () => {
    if (authToken) {
      try {
        await fetch('/api/v1/auth/logout', { method: 'POST', headers: getAuthHeaders() });
      } catch (_) {}
    }
    localStorage.removeItem('spine_token');
    setAuthToken('');
    setCurrentUser(null);
    fetchRepos();
  };

  // Create Issue
  const handleCreateIssue = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!selectedRepo) return;
    try {
      const res = await fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/issues`, {
        method: 'POST',
        headers: { ...getAuthHeaders(), 'Content-Type': 'application/json' },
        body: JSON.stringify({ title: newIssueForm.title, body: newIssueForm.body }),
      });
      if (!res.ok) throw new Error('Failed to create issue');
      setShowNewIssueModal(false);
      setNewIssueForm({ title: '', body: '' });
      fetchIssues(selectedRepo.owner, selectedRepo.name);
    } catch (err: any) {
      alert(err.message);
    }
  };

  // Add Comment on Issue
  const handleAddIssueComment = async () => {
    if (!selectedRepo || !selectedIssue || !newCommentBody.trim()) return;
    try {
      const res = await fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/issues/${selectedIssue.issue_number}/comments`, {
        method: 'POST',
        headers: { ...getAuthHeaders(), 'Content-Type': 'application/json' },
        body: JSON.stringify({ body: newCommentBody }),
      });
      if (!res.ok) throw new Error('Failed to post comment');
      setNewCommentBody('');
      handleSelectIssue(selectedIssue);
    } catch (err: any) {
      alert(err.message);
    }
  };

  // Toggle Issue Status
  const handleToggleIssueStatus = async () => {
    if (!selectedRepo || !selectedIssue) return;
    const nextStatus = selectedIssue.status === 'OPEN' ? 'CLOSED' : 'OPEN';
    try {
      const res = await fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/issues/${selectedIssue.issue_number}`, {
        method: 'PATCH',
        headers: { ...getAuthHeaders(), 'Content-Type': 'application/json' },
        body: JSON.stringify({ status: nextStatus }),
      });
      if (!res.ok) throw new Error('Failed to update issue');
      const updated = await res.json();
      setSelectedIssue(updated);
      fetchIssues(selectedRepo.owner, selectedRepo.name);
    } catch (err: any) {
      alert(err.message);
    }
  };

  // Create Pull Request
  const handleCreatePullRequest = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!selectedRepo) return;
    try {
      const res = await fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/pulls`, {
        method: 'POST',
        headers: { ...getAuthHeaders(), 'Content-Type': 'application/json' },
        body: JSON.stringify({
          title: newPrForm.title,
          body: newPrForm.body,
          base_branch: newPrForm.base_branch,
          head_branch: newPrForm.head_branch,
        }),
      });
      if (!res.ok) {
        const err = await res.json();
        throw new Error(err.message || 'Failed to create PR');
      }
      setShowNewPrModal(false);
      setNewPrForm({ title: '', body: '', base_branch: 'main', head_branch: '' });
      fetchPullRequests(selectedRepo.owner, selectedRepo.name);
    } catch (err: any) {
      alert(err.message);
    }
  };

  // Submit Review
  const handleSubmitReview = async () => {
    if (!selectedRepo || !selectedPr) return;
    try {
      const res = await fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/pulls/${selectedPr.pr_number}/reviews`, {
        method: 'POST',
        headers: { ...getAuthHeaders(), 'Content-Type': 'application/json' },
        body: JSON.stringify({ state: reviewState, body: reviewBody }),
      });
      if (!res.ok) throw new Error('Failed to submit review');
      setReviewBody('');
      handleSelectPr(selectedPr);
    } catch (err: any) {
      alert(err.message);
    }
  };

  // Execute Native Merge
  const handleExecuteMerge = async () => {
    if (!selectedRepo || !selectedPr) return;
    setMergeStatusMsg('Merging...');
    try {
      const res = await fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/pulls/${selectedPr.pr_number}/merge`, {
        method: 'POST',
        headers: { ...getAuthHeaders(), 'Content-Type': 'application/json' },
        body: JSON.stringify({}),
      });
      const data = await res.json();
      if (!res.ok) throw new Error(data.message || 'Merge failed');
      setMergeStatusMsg('Merge successful! Branch reference updated atomically.');
      fetchPullRequests(selectedRepo.owner, selectedRepo.name);
      // Reload PR details
      handleSelectPr({ ...selectedPr, state: 'MERGED' });
    } catch (err: any) {
      setMergeStatusMsg(`Merge error: ${err.message}`);
    }
  };

  // Add Inline Comment
  const handleAddInlineComment = async () => {
    if (!selectedRepo || !selectedPr || !inlineCommentForm || !inlineCommentForm.body.trim()) return;
    try {
      const res = await fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/pulls/${selectedPr.pr_number}/comments`, {
        method: 'POST',
        headers: { ...getAuthHeaders(), 'Content-Type': 'application/json' },
        body: JSON.stringify({
          file_path: inlineCommentForm.path,
          line_number: inlineCommentForm.line,
          diff_side: 'RIGHT',
          body: inlineCommentForm.body,
        }),
      });
      if (!res.ok) throw new Error('Failed to post inline comment');
      setInlineCommentForm(null);
      handleSelectPr(selectedPr);
    } catch (err: any) {
      alert(err.message);
    }
  };

  // Resolve Inline Comment
  const handleResolveInlineComment = async (commentId: string, currentResolved: boolean) => {
    if (!selectedRepo || !selectedPr) return;
    try {
      await fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/pulls/${selectedPr.pr_number}/comments/${commentId}/resolve`, {
        method: 'PATCH',
        headers: { ...getAuthHeaders(), 'Content-Type': 'application/json' },
        body: JSON.stringify({ resolved: !currentResolved }),
      });
      handleSelectPr(selectedPr);
    } catch (err: any) {
      alert(err.message);
    }
  };

  // Create Organization
  const handleCreateOrg = async (e: React.FormEvent) => {
    e.preventDefault();
    try {
      const res = await fetch('/api/v1/orgs', {
        method: 'POST',
        headers: { ...getAuthHeaders(), 'Content-Type': 'application/json' },
        body: JSON.stringify(newOrgForm),
      });
      if (!res.ok) throw new Error('Failed to create organization');
      setShowNewOrgModal(false);
      setNewOrgForm({ name: '', display_name: '', description: '' });
      fetchOrgs();
    } catch (err: any) {
      alert(err.message);
    }
  };

  // Create Team
  const handleCreateTeam = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!selectedOrg) return;
    try {
      const res = await fetch(`/api/v1/orgs/${selectedOrg.name}/teams`, {
        method: 'POST',
        headers: { ...getAuthHeaders(), 'Content-Type': 'application/json' },
        body: JSON.stringify(newTeamForm),
      });
      if (!res.ok) throw new Error('Failed to create team');
      setShowNewTeamModal(false);
      setNewTeamForm({ name: '', description: '' });
      const teamsRes = await fetch(`/api/v1/orgs/${selectedOrg.name}/teams`, { headers: getAuthHeaders() });
      if (teamsRes.ok) setOrgTeams(await teamsRes.json());
    } catch (err: any) {
      alert(err.message);
    }
  };

  // Create Branch Protection Rule
  const handleCreateProtectionRule = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!selectedRepo) return;
    try {
      const res = await fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/protection`, {
        method: 'POST',
        headers: { ...getAuthHeaders(), 'Content-Type': 'application/json' },
        body: JSON.stringify(newRuleForm),
      });
      if (!res.ok) throw new Error('Failed to set branch protection');
      setShowNewRuleModal(false);
      // Reload rules
      const rRes = await fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/protection`, {
        headers: getAuthHeaders(),
      });
      if (rRes.ok) setProtectionRules(await rRes.json());
    } catch (err: any) {
      alert(err.message);
    }
  };

  // Mark all notifications read
  const handleMarkAllNotificationsRead = async () => {
    try {
      await fetch('/api/v1/notifications/read-all', { method: 'POST', headers: getAuthHeaders() });
      setNotifications([]);
    } catch (_) {}
  };

  return (
    <div className="container">
      {/* Top Navbar */}
      <header className="header">
        <div className="logo" onClick={() => { setActiveTab('code'); setSelectedIssue(null); setSelectedPr(null); }}>
          <span style={{ fontSize: '1.8rem' }}>⚡</span>
          <span>SPINE FORGE</span>
          <span className="badge badge-ready">v0.9</span>
        </div>

        <div className="nav-actions">
          {/* Notification Bell */}
          {currentUser && (
            <button className="notif-bell-btn" onClick={() => setShowNotificationsModal(true)} title="Notifications">
              🔔
              {notifications.length > 0 && <span className="notif-count">{notifications.length}</span>}
            </button>
          )}

          {/* User Status / Login */}
          {currentUser ? (
            <div style={{ display: 'flex', alignItems: 'center', gap: '0.75rem' }}>
              <span style={{ fontSize: '0.9rem', color: '#c9d1d9' }}>
                Signed in as <strong>{currentUser.display_name || currentUser.username}</strong>
              </span>
              <button className="btn btn-secondary" onClick={handleLogout}>
                Log out
              </button>
            </div>
          ) : (
            <div style={{ display: 'flex', gap: '0.5rem' }}>
              <button
                className="btn btn-secondary"
                onClick={() => {
                  setAuthMode('login');
                  setShowAuthModal(true);
                }}
              >
                Sign in
              </button>
              <button
                className="btn btn-primary"
                onClick={() => {
                  setAuthMode('register');
                  setShowAuthModal(true);
                }}
              >
                Sign up
              </button>
            </div>
          )}
        </div>
      </header>

      {/* Main Navigation Bar */}
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '1.5rem', borderBottom: '1px solid #30363d', paddingBottom: '0.5rem' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: '1rem' }}>
          {/* Repository Selector */}
          <select
            className="form-input"
            style={{ width: 'auto', minWidth: '220px', fontWeight: 600 }}
            value={selectedRepo ? `${selectedRepo.owner}/${selectedRepo.name}` : ''}
            onChange={(e) => {
              const r = repos.find((repo) => `${repo.owner}/${repo.name}` === e.target.value);
              if (r) setSelectedRepo(r);
            }}
          >
            {repos.map((r) => (
              <option key={r.id} value={`${r.owner}/${r.name}`}>
                {r.owner}/{r.name} {r.is_private ? '🔒' : ''}
              </option>
            ))}
          </select>
          {loading && <span style={{ color: '#8b949e', fontSize: '0.8rem' }}>Loading repos...</span>}

          {/* Tabs */}
          <div style={{ display: 'flex', gap: '0.25rem' }}>
            <button
              className={`btn ${activeTab === 'code' ? 'btn-primary' : 'btn-secondary'}`}
              onClick={() => { setActiveTab('code'); setSelectedIssue(null); setSelectedPr(null); }}
            >
              💻 Code
            </button>
            <button
              className={`btn ${activeTab === 'issues' ? 'btn-primary' : 'btn-secondary'}`}
              onClick={() => { setActiveTab('issues'); setSelectedIssue(null); }}
            >
              🐞 Issues {issues.length > 0 && <span className="badge badge-open" style={{ marginLeft: '4px' }}>{issues.filter(i => i.status === 'OPEN').length}</span>}
            </button>
            <button
              className={`btn ${activeTab === 'pulls' ? 'btn-primary' : 'btn-secondary'}`}
              onClick={() => { setActiveTab('pulls'); setSelectedPr(null); }}
            >
              🔀 Pull Requests {pullRequests.length > 0 && <span className="badge badge-open" style={{ marginLeft: '4px' }}>{pullRequests.filter(p => p.state === 'OPEN').length}</span>}
            </button>
            <button
              className={`btn ${activeTab === 'orgs' ? 'btn-primary' : 'btn-secondary'}`}
              onClick={() => setActiveTab('orgs')}
            >
              🏢 Organizations
            </button>
            <button
              className={`btn ${activeTab === 'protection' ? 'btn-primary' : 'btn-secondary'}`}
              onClick={() => setActiveTab('protection')}
            >
              🛡️ Protection
            </button>
            <button
              className={`btn ${activeTab === 'releases' ? 'btn-primary' : 'btn-secondary'}`}
              onClick={() => setActiveTab('releases')}
            >
              🏷️ Releases {releases.length > 0 && <span className="badge badge-open" style={{ marginLeft: '4px' }}>{releases.length}</span>}
            </button>
            <button
              className={`btn ${activeTab === 'packages' ? 'btn-primary' : 'btn-secondary'}`}
              onClick={() => setActiveTab('packages')}
            >
              📦 Packages {packages.length > 0 && <span className="badge badge-open" style={{ marginLeft: '4px' }}>{packages.length}</span>}
            </button>
            <button
              className={`btn ${activeTab === 'actions' ? 'btn-primary' : 'btn-secondary'}`}
              onClick={() => { setActiveTab('actions'); setSelectedRun(null); }}
            >
              ⚙️ Actions {workflowRuns.length > 0 && <span className="badge badge-open" style={{ marginLeft: '4px' }}>{workflowRuns.length}</span>}
            </button>
            <button
              className={`btn ${activeTab === 'webhooks' ? 'btn-primary' : 'btn-secondary'}`}
              onClick={() => setActiveTab('webhooks')}
            >
              🔗 Webhooks
            </button>
            <button
              className={`btn ${activeTab === 'runners' ? 'btn-primary' : 'btn-secondary'}`}
              onClick={() => setActiveTab('runners')}
            >
              🏃 Runners
            </button>
            <button
              className={`btn ${activeTab === 'secrets' ? 'btn-primary' : 'btn-secondary'}`}
              onClick={() => setActiveTab('secrets')}
            >
              🔑 Secrets
            </button>
          </div>
        </div>

        <div>
          <button className="btn btn-secondary" onClick={() => setShowNewRepoModal(true)}>
            + New Repository
          </button>
        </div>
      </div>

      {/* ---------------------------------------------------------------------- */}
      {/* TAB 1: CODE (Files & Commits)                                          */}
      {/* ---------------------------------------------------------------------- */}
      {activeTab === 'code' && selectedRepo && (
        <div>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '1rem' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: '0.5rem' }}>
              <span>🌿 Branch:</span>
              <select
                className="form-input"
                style={{ width: 'auto' }}
                value={currentBranch}
                onChange={(e) => {
                  setCurrentBranch(e.target.value);
                  fetchTree(selectedRepo.owner, selectedRepo.name, e.target.value, '');
                }}
              >
                {branches.map((b) => (
                  <option key={b.name} value={b.name}>
                    {b.name}
                  </option>
                ))}
              </select>
            </div>

            <div style={{ display: 'flex', gap: '0.5rem' }}>
              <button
                className={`btn ${codeSubTab === 'files' ? 'btn-primary' : 'btn-secondary'}`}
                onClick={() => setCodeSubTab('files')}
              >
                Files
              </button>
              <button
                className={`btn ${codeSubTab === 'commits' ? 'btn-primary' : 'btn-secondary'}`}
                onClick={() => setCodeSubTab('commits')}
              >
                Commits ({commits.length})
              </button>
            </div>
          </div>

          {codeSubTab === 'files' && (
            <div>
              {currentPath && (
                <div style={{ marginBottom: '0.5rem' }}>
                  <button
                    className="btn btn-secondary"
                    style={{ fontSize: '0.8rem', padding: '0.2rem 0.5rem' }}
                    onClick={() => {
                      const parent = currentPath.includes('/') ? currentPath.substring(0, currentPath.lastIndexOf('/')) : '';
                      setCurrentPath(parent);
                      fetchTree(selectedRepo.owner, selectedRepo.name, currentBranch, parent);
                    }}
                  >
                    ⬆️ Parent directory ({currentPath})
                  </button>
                </div>
              )}

              <div className="diff-container">
                <div className="diff-header">
                  <span>📂 {currentPath || '/'}</span>
                  <span>{treeEntries.length} items</span>
                </div>
                <div style={{ padding: '0.5rem' }}>
                  {treeEntries.map((e) => (
                    <div
                      key={e.path}
                      onClick={() => handleFileClick(e)}
                      style={{
                        padding: '0.4rem 0.6rem',
                        cursor: 'pointer',
                        display: 'flex',
                        alignItems: 'center',
                        gap: '0.5rem',
                        borderBottom: '1px solid #21262d',
                      }}
                    >
                      <span>{e.is_dir ? '📁' : '📄'}</span>
                      <span style={{ color: e.is_dir ? '#58a6ff' : '#c9d1d9', fontWeight: e.is_dir ? 600 : 400 }}>
                        {e.name}
                      </span>
                    </div>
                  ))}
                  {treeEntries.length === 0 && <div style={{ color: '#8b949e', padding: '1rem' }}>No files found in this directory.</div>}
                </div>
              </div>

              {selectedBlob && (
                <div className="diff-container" style={{ marginTop: '1rem' }}>
                  <div className="diff-header">
                    <span>📄 {selectedBlob.path}</span>
                    <span>{selectedBlob.size} bytes</span>
                  </div>
                  <pre className="blob-content" style={{ padding: '1rem' }}>
                    {selectedBlob.is_binary ? 'Binary file (cannot preview)' : selectedBlob.content}
                  </pre>
                </div>
              )}
            </div>
          )}

          {codeSubTab === 'commits' && (
            <div>
              {commits.map((c) => (
                <div key={c.id} className="commit-card">
                  <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                    <span className="commit-msg">{c.message}</span>
                    <span className="commit-hash">{c.id.substring(0, 10)}</span>
                  </div>
                  <div className="commit-meta">
                    {c.author_name} committed on {new Date(c.timestamp_nanos / 1_000_000).toLocaleString()}
                  </div>
                </div>
              ))}
              {commits.length === 0 && <div style={{ color: '#8b949e', padding: '2rem', textAlign: 'center' }}>No commits on branch {currentBranch}</div>}
            </div>
          )}
        </div>
      )}

      {/* ---------------------------------------------------------------------- */}
      {/* TAB 2: ISSUES                                                          */}
      {/* ---------------------------------------------------------------------- */}
      {activeTab === 'issues' && selectedRepo && (
        <div>
          {!selectedIssue ? (
            <div>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '1rem' }}>
                <div style={{ display: 'flex', gap: '0.5rem' }}>
                  <button
                    className={`btn ${issueFilter === 'OPEN' ? 'btn-primary' : 'btn-secondary'}`}
                    onClick={() => setIssueFilter('OPEN')}
                  >
                    Open ({issues.filter((i) => i.status === 'OPEN').length})
                  </button>
                  <button
                    className={`btn ${issueFilter === 'CLOSED' ? 'btn-primary' : 'btn-secondary'}`}
                    onClick={() => setIssueFilter('CLOSED')}
                  >
                    Closed ({issues.filter((i) => i.status === 'CLOSED').length})
                  </button>
                </div>
                <button className="btn btn-primary" onClick={() => setShowNewIssueModal(true)}>
                  New Issue
                </button>
              </div>

              <div className="diff-container">
                <div className="diff-header">
                  <span>{issueFilter} Issues</span>
                </div>
                <div>
                  {issues
                    .filter((i) => i.status === issueFilter)
                    .map((iss) => (
                      <div
                        key={iss.id}
                        onClick={() => handleSelectIssue(iss)}
                        style={{
                          padding: '0.8rem 1rem',
                          borderBottom: '1px solid #30363d',
                          cursor: 'pointer',
                          display: 'flex',
                          justifyContent: 'space-between',
                          alignItems: 'center',
                        }}
                      >
                        <div>
                          <div style={{ fontWeight: 600, fontSize: '1rem', color: '#e6edf3', marginBottom: '0.2rem' }}>
                            <span style={{ color: '#8b949e', marginRight: '0.5rem' }}>#{iss.issue_number}</span>
                            {iss.title}
                          </div>
                          <div style={{ fontSize: '0.8rem', color: '#8b949e' }}>
                            opened by {iss.author.username} · {iss.comments_count} comments
                            {iss.labels.map((l) => (
                              <span key={l.id} className="tag-pill" style={{ backgroundColor: l.color, marginLeft: '0.5rem' }}>
                                {l.name}
                              </span>
                            ))}
                          </div>
                        </div>
                        <span className={`badge ${iss.status === 'OPEN' ? 'badge-open' : 'badge-closed'}`}>
                          {iss.status}
                        </span>
                      </div>
                    ))}
                  {issues.filter((i) => i.status === issueFilter).length === 0 && (
                    <div style={{ padding: '2rem', textAlign: 'center', color: '#8b949e' }}>
                      No {issueFilter.toLowerCase()} issues found.
                    </div>
                  )}
                </div>
              </div>
            </div>
          ) : (
            // Issue Detail View
            <div>
              <button
                className="btn btn-secondary"
                style={{ marginBottom: '1rem' }}
                onClick={() => setSelectedIssue(null)}
              >
                ← Back to issues
              </button>

              <div style={{ borderBottom: '1px solid #30363d', paddingBottom: '1rem', marginBottom: '1.5rem' }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: '0.75rem', marginBottom: '0.5rem' }}>
                  <h2 style={{ margin: 0 }}>{selectedIssue.title}</h2>
                  <span style={{ color: '#8b949e', fontSize: '1.4rem' }}>#{selectedIssue.issue_number}</span>
                  <span className={`badge ${selectedIssue.status === 'OPEN' ? 'badge-open' : 'badge-closed'}`}>
                    {selectedIssue.status}
                  </span>
                </div>
                <div style={{ color: '#8b949e', fontSize: '0.85rem' }}>
                  {selectedIssue.author.username} opened this issue · {issueComments.length} comments
                </div>
              </div>

              {/* Main Issue Description */}
              <div className="comment-card">
                <div className="comment-card-header">
                  <strong>{selectedIssue.author.username}</strong>
                  <span>{new Date(selectedIssue.created_at_nanos / 1_000_000).toLocaleString()}</span>
                </div>
                <div className="comment-card-body">{selectedIssue.body || 'No description provided.'}</div>
              </div>

              {/* Comments Thread */}
              {issueComments.map((c) => (
                <div key={c.id} className="comment-card">
                  <div className="comment-card-header">
                    <strong>{c.author.username}</strong>
                    <span>{new Date(c.created_at_nanos / 1_000_000).toLocaleString()}</span>
                  </div>
                  <div className="comment-card-body">{c.body}</div>
                </div>
              ))}

              {/* Comment Input */}
              <div style={{ marginTop: '1.5rem' }}>
                <textarea
                  className="form-input"
                  rows={4}
                  placeholder="Leave a comment..."
                  value={newCommentBody}
                  onChange={(e) => setNewCommentBody(e.target.value)}
                  style={{ marginBottom: '0.5rem' }}
                />
                <div style={{ display: 'flex', justifyContent: 'space-between' }}>
                  <button className="btn btn-secondary" onClick={handleToggleIssueStatus}>
                    {selectedIssue.status === 'OPEN' ? 'Close issue' : 'Reopen issue'}
                  </button>
                  <button className="btn btn-primary" onClick={handleAddIssueComment}>
                    Comment
                  </button>
                </div>
              </div>
            </div>
          )}
        </div>
      )}

      {/* ---------------------------------------------------------------------- */}
      {/* TAB 3: PULL REQUESTS & NATIVE MERGES                                   */}
      {/* ---------------------------------------------------------------------- */}
      {activeTab === 'pulls' && selectedRepo && (
        <div>
          {!selectedPr ? (
            <div>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '1rem' }}>
                <div style={{ display: 'flex', gap: '0.5rem' }}>
                  <button
                    className={`btn ${prFilter === 'OPEN' ? 'btn-primary' : 'btn-secondary'}`}
                    onClick={() => setPrFilter('OPEN')}
                  >
                    Open ({pullRequests.filter((p) => p.state === 'OPEN').length})
                  </button>
                  <button
                    className={`btn ${prFilter === 'MERGED' ? 'btn-primary' : 'btn-secondary'}`}
                    onClick={() => setPrFilter('MERGED')}
                  >
                    Merged ({pullRequests.filter((p) => p.state === 'MERGED').length})
                  </button>
                  <button
                    className={`btn ${prFilter === 'CLOSED' ? 'btn-primary' : 'btn-secondary'}`}
                    onClick={() => setPrFilter('CLOSED')}
                  >
                    Closed ({pullRequests.filter((p) => p.state === 'CLOSED').length})
                  </button>
                </div>
                <button
                  className="btn btn-primary"
                  onClick={() => {
                    setNewPrForm({
                      title: '',
                      body: '',
                      base_branch: 'main',
                      head_branch: branches.find((b) => b.name !== 'main')?.name || '',
                    });
                    setShowNewPrModal(true);
                  }}
                >
                  New Pull Request
                </button>
              </div>

              <div className="diff-container">
                <div className="diff-header">
                  <span>{prFilter} Pull Requests</span>
                </div>
                <div>
                  {pullRequests
                    .filter((p) => p.state === prFilter)
                    .map((pr) => (
                      <div
                        key={pr.id}
                        onClick={() => handleSelectPr(pr)}
                        style={{
                          padding: '0.8rem 1rem',
                          borderBottom: '1px solid #30363d',
                          cursor: 'pointer',
                          display: 'flex',
                          justifyContent: 'space-between',
                          alignItems: 'center',
                        }}
                      >
                        <div>
                          <div style={{ fontWeight: 600, fontSize: '1rem', color: '#e6edf3', marginBottom: '0.2rem' }}>
                            <span style={{ color: '#8b949e', marginRight: '0.5rem' }}>#{pr.pr_number}</span>
                            {pr.title}
                          </div>
                          <div style={{ fontSize: '0.8rem', color: '#8b949e' }}>
                            {pr.head_branch} → {pr.base_branch} by {pr.author.username}
                          </div>
                        </div>
                        <span
                          className={`badge ${
                            pr.state === 'OPEN' ? 'badge-open' : pr.state === 'MERGED' ? 'badge-merged' : 'badge-closed'
                          }`}
                        >
                          {pr.state}
                        </span>
                      </div>
                    ))}
                  {pullRequests.filter((p) => p.state === prFilter).length === 0 && (
                    <div style={{ padding: '2rem', textAlign: 'center', color: '#8b949e' }}>
                      No {prFilter.toLowerCase()} pull requests.
                    </div>
                  )}
                </div>
              </div>
            </div>
          ) : (
            // PR Detail View
            <div>
              <button className="btn btn-secondary" style={{ marginBottom: '1rem' }} onClick={() => setSelectedPr(null)}>
                ← Back to pull requests
              </button>

              <div style={{ borderBottom: '1px solid #30363d', paddingBottom: '1rem', marginBottom: '1rem' }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: '0.75rem', marginBottom: '0.5rem' }}>
                  <h2 style={{ margin: 0 }}>{selectedPr.title}</h2>
                  <span style={{ color: '#8b949e', fontSize: '1.4rem' }}>#{selectedPr.pr_number}</span>
                  <span
                    className={`badge ${
                      selectedPr.state === 'OPEN' ? 'badge-open' : selectedPr.state === 'MERGED' ? 'badge-merged' : 'badge-closed'
                    }`}
                  >
                    {selectedPr.state}
                  </span>
                </div>
                <div style={{ color: '#8b949e', fontSize: '0.85rem' }}>
                  <strong>{selectedPr.author.username}</strong> wants to merge into{' '}
                  <code style={{ color: '#58a6ff' }}>{selectedPr.base_branch}</code> from{' '}
                  <code style={{ color: '#58a6ff' }}>{selectedPr.head_branch}</code>
                </div>
              </div>

              {/* PR Sub-tabs */}
              <div style={{ display: 'flex', gap: '0.5rem', marginBottom: '1.5rem' }}>
                <button
                  className={`btn ${prSubTab === 'conversation' ? 'btn-primary' : 'btn-secondary'}`}
                  onClick={() => setPrSubTab('conversation')}
                >
                  💬 Conversation
                </button>
                <button
                  className={`btn ${prSubTab === 'diff' ? 'btn-primary' : 'btn-secondary'}`}
                  onClick={() => setPrSubTab('diff')}
                >
                  📝 Files Changed ({prDiff ? prDiff.files.length : 0})
                </button>
              </div>

              {/* Sub-tab 1: Conversation & Review Actions */}
              {prSubTab === 'conversation' && (
                <div>
                  <div className="comment-card">
                    <div className="comment-card-header">
                      <strong>{selectedPr.author.username}</strong>
                      <span>{new Date(selectedPr.created_at_nanos / 1_000_000).toLocaleString()}</span>
                    </div>
                    <div className="comment-card-body">{selectedPr.body || 'No description provided.'}</div>
                  </div>

                  {/* Reviews List */}
                  {prReviews.map((rev) => (
                    <div key={rev.id} className="comment-card">
                      <div className="comment-card-header">
                        <div>
                          <strong>{rev.reviewer.username}</strong> reviewed:{' '}
                          <span
                            className={
                              rev.state === 'APPROVED'
                                ? 'review-badge-approved'
                                : rev.state === 'CHANGES_REQUESTED'
                                ? 'review-badge-changes'
                                : 'review-badge-commented'
                            }
                          >
                            {rev.state}
                          </span>
                        </div>
                        <span>{new Date(rev.created_at_nanos / 1_000_000).toLocaleString()}</span>
                      </div>
                      {rev.body && <div className="comment-card-body">{rev.body}</div>}
                    </div>
                  ))}

                  {/* Submit Review Card */}
                  {selectedPr.state === 'OPEN' && (
                    <div className="merge-box" style={{ marginBottom: '1.5rem' }}>
                      <h4 style={{ margin: '0 0 0.5rem 0' }}>Submit a Code Review</h4>
                      <div style={{ display: 'flex', gap: '1rem', marginBottom: '0.75rem' }}>
                        <label>
                          <input
                            type="radio"
                            name="revState"
                            checked={reviewState === 'APPROVED'}
                            onChange={() => setReviewState('APPROVED')}
                          />{' '}
                          ✅ Approve
                        </label>
                        <label>
                          <input
                            type="radio"
                            name="revState"
                            checked={reviewState === 'CHANGES_REQUESTED'}
                            onChange={() => setReviewState('CHANGES_REQUESTED')}
                          />{' '}
                          ❌ Request Changes
                        </label>
                        <label>
                          <input
                            type="radio"
                            name="revState"
                            checked={reviewState === 'COMMENTED'}
                            onChange={() => setReviewState('COMMENTED')}
                          />{' '}
                          💬 Comment Only
                        </label>
                      </div>
                      <textarea
                        className="form-input"
                        rows={3}
                        placeholder="Review comments..."
                        value={reviewBody}
                        onChange={(e) => setReviewBody(e.target.value)}
                        style={{ marginBottom: '0.75rem' }}
                      />
                      <button className="btn btn-secondary" onClick={handleSubmitReview}>
                        Submit Review
                      </button>
                    </div>
                  )}

                  {/* Native Merge Box */}
                  <div
                    className={`merge-box ${
                      prMergeability && prMergeability.mergeable ? 'merge-box-clean' : 'merge-box-conflict'
                    }`}
                  >
                    <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                      <div>
                        <h4 style={{ margin: '0 0 0.35rem 0' }}>
                          {selectedPr.state === 'MERGED'
                            ? '💜 Merged'
                            : prMergeability && prMergeability.mergeable
                            ? '✅ This branch has no conflicts with the base branch'
                            : '⚠️ Merging is blocked'}
                        </h4>
                        <div style={{ fontSize: '0.85rem', color: '#8b949e' }}>
                          {selectedPr.state === 'MERGED'
                            ? `Merged commit: ${selectedPr.merge_commit_id}`
                            : prMergeability && prMergeability.mergeable
                            ? 'All branch protection checks passed. Ready for atomic 3-way merge.'
                            : `Conflicts in: ${prMergeability ? prMergeability.conflict_files.join(', ') : 'resolving'}`}
                        </div>
                      </div>

                      {selectedPr.state === 'OPEN' && prMergeability && prMergeability.mergeable && (
                        <button className="btn btn-primary" onClick={handleExecuteMerge}>
                          Merge Pull Request
                        </button>
                      )}
                    </div>
                    {mergeStatusMsg && (
                      <div style={{ marginTop: '0.75rem', fontSize: '0.85rem', color: '#58a6ff' }}>{mergeStatusMsg}</div>
                    )}
                  </div>
                </div>
              )}

              {/* Sub-tab 2: Diff Viewer & Inline Comments */}
              {prSubTab === 'diff' && prDiff && (
                <div>
                  <div style={{ display: 'flex', gap: '1rem', marginBottom: '1rem', fontSize: '0.9rem', color: '#8b949e' }}>
                    <span>Showing {prDiff.files.length} changed files</span>
                    <span style={{ color: '#3fb950' }}>+{prDiff.total_additions} additions</span>
                    <span style={{ color: '#f85149' }}>-{prDiff.total_deletions} deletions</span>
                  </div>

                  {prDiff.files.map((file) => (
                    <div key={file.path} className="diff-container">
                      <div className="diff-header">
                        <span>{file.path} ({file.change_type})</span>
                        <div>
                          <span style={{ color: '#3fb950', marginRight: '0.5rem' }}>+{file.additions}</span>
                          <span style={{ color: '#f85149' }}>-{file.deletions}</span>
                        </div>
                      </div>

                      <div className="diff-body">
                        {file.patch.split('\n').map((line, idx) => {
                          const isAdd = line.startsWith('+');
                          const isDel = line.startsWith('-');
                          const lineClass = isAdd ? 'diff-line-add' : isDel ? 'diff-line-del' : 'diff-line-context';

                          return (
                            <div key={idx} className={`diff-line ${lineClass}`} style={{ position: 'relative' }}>
                              <span style={{ width: '40px', color: '#6e7681', userSelect: 'none' }}>{idx + 1}</span>
                              <span style={{ flex: 1 }}>{line}</span>
                              {selectedPr.state === 'OPEN' && (
                                <button
                                  style={{
                                    fontSize: '0.65rem',
                                    padding: '0 0.3rem',
                                    background: '#21262d',
                                    border: '1px solid #30363d',
                                    color: '#c9d1d9',
                                    cursor: 'pointer',
                                    borderRadius: '3px',
                                  }}
                                  onClick={() => setInlineCommentForm({ path: file.path, line: idx + 1, body: '' })}
                                  title="Add inline review comment"
                                >
                                  +
                                </button>
                              )}
                            </div>
                          );
                        })}
                      </div>

                      {/* Display inline review comments for this file */}
                      {prComments
                        .filter((c) => c.file_path === file.path)
                        .map((c) => (
                          <div
                            key={c.id}
                            style={{
                              margin: '0.5rem 1rem',
                              padding: '0.6rem',
                              backgroundColor: '#161b22',
                              border: '1px solid #30363d',
                              borderRadius: '4px',
                            }}
                          >
                            <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: '0.8rem', color: '#8b949e' }}>
                              <span>
                                <strong>{c.author.username}</strong> on line {c.line_number}
                              </span>
                              <button
                                className="btn btn-secondary"
                                style={{ fontSize: '0.7rem', padding: '0.1rem 0.4rem' }}
                                onClick={() => handleResolveInlineComment(c.id, c.is_resolved)}
                              >
                                {c.is_resolved ? '✅ Resolved' : '⭕ Unresolved'}
                              </button>
                            </div>
                            <div style={{ marginTop: '0.4rem', fontSize: '0.85rem' }}>{c.body}</div>
                          </div>
                        ))}

                      {/* Inline comment form */}
                      {inlineCommentForm && inlineCommentForm.path === file.path && (
                        <div style={{ padding: '0.8rem', backgroundColor: '#161b22', borderTop: '1px solid #30363d' }}>
                          <div style={{ fontSize: '0.8rem', marginBottom: '0.4rem', color: '#58a6ff' }}>
                            Add comment on line {inlineCommentForm.line}:
                          </div>
                          <input
                            type="text"
                            className="form-input"
                            placeholder="Write inline comment..."
                            value={inlineCommentForm.body}
                            onChange={(e) => setInlineCommentForm({ ...inlineCommentForm, body: e.target.value })}
                            style={{ marginBottom: '0.5rem' }}
                          />
                          <div style={{ display: 'flex', gap: '0.5rem' }}>
                            <button className="btn btn-primary" onClick={handleAddInlineComment}>
                              Save Comment
                            </button>
                            <button className="btn btn-secondary" onClick={() => setInlineCommentForm(null)}>
                              Cancel
                            </button>
                          </div>
                        </div>
                      )}
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>
      )}

      {/* ---------------------------------------------------------------------- */}
      {/* TAB 4: ORGANIZATIONS & TEAMS                                           */}
      {/* ---------------------------------------------------------------------- */}
      {activeTab === 'orgs' && (
        <div>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '1rem' }}>
            <div style={{ display: 'flex', gap: '0.5rem', alignItems: 'center' }}>
              <span>🏢 Organization:</span>
              <select
                className="form-input"
                style={{ width: 'auto' }}
                value={selectedOrg ? selectedOrg.name : ''}
                onChange={(e) => {
                  const o = orgs.find((org) => org.name === e.target.value);
                  if (o) setSelectedOrg(o);
                }}
              >
                {orgs.map((o) => (
                  <option key={o.id} value={o.name}>
                    {o.display_name} ({o.name})
                  </option>
                ))}
              </select>
            </div>
            <button className="btn btn-primary" onClick={() => setShowNewOrgModal(true)}>
              + New Organization
            </button>
          </div>

          {selectedOrg ? (
            <div>
              <div className="diff-container" style={{ padding: '1.25rem', marginBottom: '1.5rem' }}>
                <h3 style={{ margin: '0 0 0.5rem 0' }}>{selectedOrg.display_name}</h3>
                <p style={{ color: '#8b949e', margin: 0 }}>{selectedOrg.description || 'No description provided.'}</p>
              </div>

              {/* Teams List */}
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '0.75rem' }}>
                <h4>Teams</h4>
                <button className="btn btn-secondary" onClick={() => setShowNewTeamModal(true)}>
                  + Add Team
                </button>
              </div>

              <div className="diff-container">
                <div className="diff-header">
                  <span>Organization Teams</span>
                </div>
                <div style={{ padding: '0.75rem' }}>
                  {orgTeams.map((t) => (
                    <div
                      key={t.id}
                      style={{
                        padding: '0.5rem',
                        borderBottom: '1px solid #21262d',
                        display: 'flex',
                        justifyContent: 'space-between',
                      }}
                    >
                      <div>
                        <strong>{t.name}</strong> <span style={{ color: '#8b949e' }}>@{t.normalized_slug}</span>
                        <div style={{ fontSize: '0.8rem', color: '#8b949e' }}>{t.description}</div>
                      </div>
                    </div>
                  ))}
                  {orgTeams.length === 0 && <div style={{ color: '#8b949e', padding: '1rem' }}>No teams created yet.</div>}
                </div>
              </div>

              {/* Members List */}
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '0.75rem', marginTop: '1.5rem' }}>
                <h4>Members ({orgMembers.length})</h4>
              </div>

              <div className="diff-container">
                <div className="diff-header">
                  <span>Organization Members</span>
                </div>
                <div style={{ padding: '0.75rem' }}>
                  {orgMembers.map((m) => (
                    <div
                      key={m.user.id}
                      style={{
                        padding: '0.5rem',
                        borderBottom: '1px solid #21262d',
                        display: 'flex',
                        justifyContent: 'space-between',
                        alignItems: 'center',
                      }}
                    >
                      <div>
                        <strong>{m.user.username}</strong>
                      </div>
                      <span className="badge badge-open">{m.role}</span>
                    </div>
                  ))}
                  {orgMembers.length === 0 && <div style={{ color: '#8b949e', padding: '1rem' }}>No members found.</div>}
                </div>
              </div>
            </div>
          ) : (
            <div style={{ textAlign: 'center', padding: '3rem', color: '#8b949e' }}>
              You do not belong to any organizations. Create one above!
            </div>
          )}
        </div>
      )}

      {/* ---------------------------------------------------------------------- */}
      {/* TAB 5: BRANCH PROTECTION RULES                                         */}
      {/* ---------------------------------------------------------------------- */}
      {activeTab === 'protection' && selectedRepo && (
        <div>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '1rem' }}>
            <h3>Branch Protection Rules</h3>
            <button className="btn btn-primary" onClick={() => setShowNewRuleModal(true)}>
              + Add Rule
            </button>
          </div>

          <div className="diff-container">
            <div className="diff-header">
              <span>Protected Branch Patterns for {selectedRepo.name}</span>
            </div>
            <div style={{ padding: '0.75rem' }}>
              {protectionRules.map((rule) => (
                <div
                  key={rule.id}
                  style={{
                    padding: '0.8rem',
                    borderBottom: '1px solid #30363d',
                    display: 'flex',
                    justifyContent: 'space-between',
                    alignItems: 'center',
                  }}
                >
                  <div>
                    <div style={{ fontWeight: 600, fontSize: '1rem', color: '#58a6ff' }}>
                      Branch pattern: <code>{rule.branch_pattern}</code>
                    </div>
                    <div style={{ fontSize: '0.85rem', color: '#8b949e', marginTop: '0.25rem' }}>
                      ✅ Require PR · {rule.required_approvals} approval(s) required · Prevent self-approval:{' '}
                      {rule.prevent_self_approval ? 'Yes' : 'No'}
                    </div>
                  </div>
                </div>
              ))}
              {protectionRules.length === 0 && (
                <div style={{ color: '#8b949e', padding: '1.5rem', textAlign: 'center' }}>
                  No branch protection rules configured for this repository.
                </div>
              )}
            </div>
          </div>
        </div>
      )}

      {/* ---------------------------------------------------------------------- */}
      {/* RELEASES TAB (Batch 11)                                                */}
      {/* ---------------------------------------------------------------------- */}
      {activeTab === 'releases' && selectedRepo && (
        <div>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '1rem' }}>
            <div>
              <h3 style={{ margin: 0 }}>Releases</h3>
              <p style={{ margin: 0, fontSize: '0.85rem', color: '#8b949e' }}>
                Immutable releases bound to tags and commits with distributed binary asset storage.
              </p>
            </div>
            <button className="btn btn-primary" onClick={() => setShowNewReleaseModal(true)}>
              + Draft a new release
            </button>
          </div>

          <div style={{ display: 'flex', flexDirection: 'column', gap: '1rem' }}>
            {releases.map((rel) => (
              <div key={rel.id} className="diff-container" style={{ padding: '1rem' }}>
                <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
                  <div>
                    <div style={{ display: 'flex', alignItems: 'center', gap: '0.5rem' }}>
                      <span className="badge badge-open" style={{ fontSize: '0.9rem' }}>🏷️ {rel.tag_name}</span>
                      <h4 style={{ margin: 0, color: '#e6edf3' }}>{rel.title}</h4>
                      {rel.is_draft && <span className="badge badge-closed">Draft</span>}
                      {rel.is_prerelease && <span className="badge" style={{ backgroundColor: '#d29922' }}>Pre-release</span>}
                    </div>
                    <div style={{ fontSize: '0.8rem', color: '#8b949e', marginTop: '0.25rem' }}>
                      Released by @{rel.author.username} · Target: <code>{rel.target_commit_id.slice(0, 8)}</code>
                    </div>
                  </div>
                </div>

                <div style={{ margin: '1rem 0', whiteSpace: 'pre-wrap', color: '#c9d1d9' }}>
                  {rel.body || <span style={{ color: '#8b949e' }}>No description provided.</span>}
                </div>

                {/* Release Assets */}
                <div style={{ borderTop: '1px solid #30363d', paddingTop: '0.75rem' }}>
                  <div style={{ fontSize: '0.85rem', fontWeight: 600, color: '#8b949e', marginBottom: '0.5rem' }}>
                    📦 Assets ({rel.assets.length})
                  </div>
                  {rel.assets.map((asset) => (
                    <div
                      key={asset.id}
                      style={{
                        display: 'flex',
                        justifyContent: 'space-between',
                        alignItems: 'center',
                        padding: '0.4rem 0',
                        fontSize: '0.85rem',
                      }}
                    >
                      <a
                        href={`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/releases/${rel.id}/assets/${asset.id}/download`}
                        style={{ color: '#58a6ff', textDecoration: 'none', fontWeight: 500 }}
                        download
                      >
                        📥 {asset.name}
                      </a>
                      <span style={{ color: '#8b949e' }}>
                        {(asset.size_bytes / (1024 * 1024)).toFixed(2)} MB · {asset.downloads_count} download(s)
                      </span>
                    </div>
                  ))}
                  {rel.assets.length === 0 && (
                    <div style={{ fontSize: '0.8rem', color: '#8b949e' }}>No binary assets attached.</div>
                  )}
                </div>
              </div>
            ))}

            {releases.length === 0 && (
              <div style={{ textAlign: 'center', padding: '3rem', color: '#8b949e' }}>
                There aren't any releases for {selectedRepo.owner}/{selectedRepo.name} yet.
              </div>
            )}
          </div>
        </div>
      )}

      {/* ---------------------------------------------------------------------- */}
      {/* PACKAGES & REGISTRY TAB (Batch 11)                                     */}
      {/* ---------------------------------------------------------------------- */}
      {activeTab === 'packages' && (
        <div>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '1rem' }}>
            <div>
              <h3 style={{ margin: 0 }}>Package Registry & Quotas</h3>
              <p style={{ margin: 0, fontSize: '0.85rem', color: '#8b949e' }}>
                Cargo-compatible sparse package registry and distributed large object storage accounting.
              </p>
            </div>
            {artifactQuota && (
              <div style={{ backgroundColor: '#161b22', padding: '0.5rem 1rem', borderRadius: '6px', border: '1px solid #30363d', fontSize: '0.85rem' }}>
                <span style={{ color: '#8b949e' }}>Quota: </span>
                <span style={{ color: '#58a6ff', fontWeight: 600 }}>
                  {(artifactQuota.used_logical_bytes / (1024 * 1024)).toFixed(2)} MB
                </span>
                <span style={{ color: '#8b949e' }}> / {(artifactQuota.logical_quota_bytes / (1024 * 1024 * 1024)).toFixed(0)} GB</span>
              </div>
            )}
          </div>

          {/* Cargo Configuration Guide Box */}
          <div className="diff-container" style={{ padding: '1rem', marginBottom: '1.5rem', backgroundColor: '#0d1117' }}>
            <h4 style={{ margin: '0 0 0.5rem 0', color: '#e6edf3' }}>🦀 Cargo Sparse Registry Configuration</h4>
            <p style={{ margin: '0 0 0.5rem 0', fontSize: '0.85rem', color: '#8b949e' }}>
              Publish and download Rust crates directly to Spine Forge using the standard Cargo CLI:
            </p>
            <pre style={{ margin: 0, padding: '0.75rem', backgroundColor: '#161b22', borderRadius: '6px', fontSize: '0.8rem', color: '#58a6ff' }}>
{`# Add to your project's .cargo/config.toml or ~/.cargo/config.toml:
[registries.spine]
index = "sparse+http://127.0.0.1:3000/index/"

# To publish:
# cargo publish --registry spine

# To depend on a crate:
# my-crate = { version = "0.1.0", registry = "spine" }`}
            </pre>
          </div>

          {/* Packages List */}
          <div className="diff-container">
            <div className="diff-header">
              <span>Published Packages ({packages.length})</span>
            </div>
            <div style={{ padding: '0.75rem' }}>
              {packages.map((pkg) => (
                <div
                  key={pkg.id}
                  style={{
                    padding: '0.75rem',
                    borderBottom: '1px solid #30363d',
                    display: 'flex',
                    justifyContent: 'space-between',
                    alignItems: 'center',
                  }}
                >
                  <div>
                    <div style={{ display: 'flex', alignItems: 'center', gap: '0.5rem' }}>
                      <span className="badge badge-open">📦 {pkg.name}</span>
                      <span style={{ fontSize: '0.85rem', color: '#8b949e' }}>v{pkg.versions[pkg.versions.length - 1]?.version || '0.1.0'}</span>
                    </div>
                    <div style={{ fontSize: '0.85rem', color: '#c9d1d9', marginTop: '0.25rem' }}>
                      {pkg.description || 'Rust crate published to Spine Forge'}
                    </div>
                  </div>
                  <div style={{ fontSize: '0.8rem', color: '#8b949e' }}>
                    {pkg.versions_count} version(s)
                  </div>
                </div>
              ))}
              {packages.length === 0 && (
                <div style={{ textAlign: 'center', padding: '2rem', color: '#8b949e' }}>
                  No packages published to this registry yet. Run <code>cargo publish --registry spine</code> to publish your first crate.
                </div>
              )}
            </div>
          </div>
        </div>
      )}

      {/* ---------------------------------------------------------------------- */}
      {/* TAB: Actions / CI/CD (Batch 12)                                        */}
      {/* ---------------------------------------------------------------------- */}
      {activeTab === 'actions' && selectedRepo && (
        <div>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '1rem' }}>
            <div>
              <h3 style={{ margin: 0 }}>⚙️ Workflows & CI/CD Automation</h3>
              <p style={{ margin: 0, fontSize: '0.85rem', color: '#8b949e' }}>
                Declarative workflows, directed acyclic graph execution, and isolated runners
              </p>
            </div>
            <div style={{ display: 'flex', gap: '0.5rem' }}>
              <button
                className="btn btn-secondary"
                onClick={async () => {
                  try {
                    await fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/actions/workflows/sync`, {
                      method: 'POST',
                      headers: { ...getAuthHeaders(), 'Content-Type': 'application/json' },
                      body: JSON.stringify({
                        files: [{
                          path: '.spine/workflows/ci.yml',
                          content: "name: CI Workflow\non:\n  push:\n    branches:\n      - main\n  pull_request:\n    branches:\n      - main\njobs:\n  build:\n    name: Build & Test\n    runs-on:\n      - self-hosted\n    steps:\n      - name: Checkout Code\n        run: echo \"Checking out repository\"\n      - name: Run Tests\n        run: echo \"Running unit tests\""
                        }]
                      })
                    });
                    fetchWorkflowsAndRuns();
                  } catch (e) {
                    alert('Sync failed: ' + e);
                  }
                }}
              >
                🔄 Sync Workflows
              </button>
              <button
                className="btn btn-primary"
                onClick={async () => {
                  if (workflows.length === 0) {
                    alert('No workflows registered. Click "Sync Workflows" first.');
                    return;
                  }
                  try {
                    await fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/actions/workflows/${workflows[0].id}/dispatches`, {
                      method: 'POST',
                      headers: { ...getAuthHeaders(), 'Content-Type': 'application/json' },
                      body: JSON.stringify({ ref_name: currentBranch })
                    });
                    fetchWorkflowsAndRuns();
                  } catch (e) {
                    alert('Dispatch failed: ' + e);
                  }
                }}
              >
                ▶ Run Workflow
              </button>
            </div>
          </div>

          {!selectedRun ? (
            <div>
              {/* Registered Workflows */}
              <div className="diff-container" style={{ marginBottom: '1.5rem' }}>
                <div className="diff-header">
                  <span>Registered Workflows ({workflows.length})</span>
                </div>
                <div style={{ padding: '0.75rem' }}>
                  {workflows.map(wf => (
                    <div key={wf.id} style={{ padding: '0.5rem', borderBottom: '1px solid #30363d', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                      <div>
                        <strong>{wf.name}</strong> <span style={{ color: '#8b949e', fontSize: '0.85rem' }}>({wf.path})</span>
                      </div>
                      <div style={{ fontSize: '0.8rem', color: '#8b949e' }}>
                        SHA-256: <code>{wf.digest.substring(0, 12)}</code>
                      </div>
                    </div>
                  ))}
                  {workflows.length === 0 && (
                    <div style={{ textAlign: 'center', padding: '1rem', color: '#8b949e' }}>
                      No workflows detected in repository. Click "Sync Workflows" to import <code>.spine/workflows/*.yml</code>.
                    </div>
                  )}
                </div>
              </div>

              {/* Workflow Runs */}
              <div className="diff-container">
                <div className="diff-header">
                  <span>Workflow Runs ({workflowRuns.length})</span>
                </div>
                <div style={{ padding: '0.75rem' }}>
                  {workflowRuns.map(run => (
                    <div
                      key={run.id}
                      onClick={() => fetchRunDetails(run.id)}
                      style={{
                        padding: '0.75rem',
                        borderBottom: '1px solid #30363d',
                        display: 'flex',
                        justifyContent: 'space-between',
                        alignItems: 'center',
                        cursor: 'pointer'
                      }}
                    >
                      <div style={{ display: 'flex', alignItems: 'center', gap: '0.75rem' }}>
                        <span className={`badge ${run.conclusion === 'SUCCESS' ? 'badge-open' : run.status === 'RUNNING' ? 'badge-ready' : 'badge-closed'}`}>
                          {run.conclusion || run.status}
                        </span>
                        <div>
                          <strong>{run.workflow_name}</strong>
                          <div style={{ fontSize: '0.8rem', color: '#8b949e', marginTop: '0.2rem' }}>
                            Trigger: <code>{run.trigger_event}</code> on <code>{run.ref_name}</code> (commit <code>{run.commit_oid.substring(0, 7)}</code>)
                          </div>
                        </div>
                      </div>
                      <div style={{ fontSize: '0.8rem', color: '#8b949e' }}>
                        {new Date(run.created_at_nanos / 1_000_000).toLocaleString()}
                      </div>
                    </div>
                  ))}
                  {workflowRuns.length === 0 && (
                    <div style={{ textAlign: 'center', padding: '2rem', color: '#8b949e' }}>
                      No workflow runs yet. Trigger a run or push commits to execute CI pipelines.
                    </div>
                  )}
                </div>
              </div>
            </div>
          ) : (
            <div>
              {/* Detailed Run View */}
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '1rem', background: '#161b22', padding: '1rem', borderRadius: '6px' }}>
                <div>
                  <button className="btn btn-secondary" style={{ marginRight: '1rem' }} onClick={() => setSelectedRun(null)}>
                    ← Back to Runs
                  </button>
                  <strong style={{ fontSize: '1.1rem' }}>{selectedRun.workflow_name}</strong>
                  <span className={`badge ${selectedRun.conclusion === 'SUCCESS' ? 'badge-open' : 'badge-closed'}`} style={{ marginLeft: '0.75rem' }}>
                    {selectedRun.conclusion || selectedRun.status}
                  </span>
                </div>
                {selectedRun.status !== 'COMPLETED' && selectedRun.status !== 'CANCELLED' && (
                  <button
                    className="btn btn-secondary"
                    onClick={async () => {
                      await fetch(`/api/v1/actions/runs/${selectedRun.id}/cancel`, { method: 'POST', headers: getAuthHeaders() });
                      fetchRunDetails(selectedRun.id);
                    }}
                  >
                    ⏹ Cancel Run
                  </button>
                )}
              </div>

              {/* Jobs and Steps/Logs Layout */}
              <div style={{ display: 'grid', gridTemplateColumns: '300px 1fr', gap: '1rem' }}>
                {/* Jobs column */}
                <div className="diff-container">
                  <div className="diff-header">
                    <span>Jobs ({runJobs.length})</span>
                  </div>
                  <div style={{ padding: '0.5rem' }}>
                    {runJobs.map(job => (
                      <div
                        key={job.id}
                        onClick={() => selectJobDetails(job)}
                        style={{
                          padding: '0.75rem',
                          borderRadius: '4px',
                          marginBottom: '0.5rem',
                          cursor: 'pointer',
                          background: selectedJob?.id === job.id ? '#1f242c' : 'transparent',
                          border: selectedJob?.id === job.id ? '1px solid #388bfd' : '1px solid #30363d'
                        }}
                      >
                        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                          <strong>{job.name}</strong>
                          <span className={`badge ${job.status === 'SUCCEEDED' ? 'badge-open' : job.status === 'RUNNING' ? 'badge-ready' : 'badge-closed'}`}>
                            {job.status}
                          </span>
                        </div>
                        <div style={{ fontSize: '0.75rem', color: '#8b949e', marginTop: '0.25rem' }}>
                          Runs-on: {job.runs_on.join(', ')}
                        </div>
                      </div>
                    ))}
                  </div>
                </div>

                {/* Steps and Logs Column */}
                <div>
                  <div className="diff-container" style={{ marginBottom: '1rem' }}>
                    <div className="diff-header">
                      <span>Steps Execution</span>
                    </div>
                    <div style={{ padding: '0.5rem' }}>
                      {jobSteps.map(step => (
                        <div key={step.id} style={{ display: 'flex', justifyContent: 'space-between', padding: '0.5rem', borderBottom: '1px solid #21262d' }}>
                          <span>Step {step.step_index + 1}: <strong>{step.name}</strong></span>
                          <span className={`badge ${step.status === 'SUCCEEDED' ? 'badge-open' : 'badge-closed'}`}>
                            {step.status} {step.exit_code !== null ? `(code ${step.exit_code})` : ''}
                          </span>
                        </div>
                      ))}
                      {jobSteps.length === 0 && (
                        <div style={{ color: '#8b949e', padding: '0.5rem' }}>Select a job to view steps.</div>
                      )}
                    </div>
                  </div>

                  <div className="diff-container">
                    <div className="diff-header">
                      <span>Live Runner Console Output</span>
                    </div>
                    <div style={{ background: '#0d1117', color: '#58a6ff', padding: '1rem', fontFamily: 'monospace', fontSize: '0.85rem', maxHeight: '400px', overflowY: 'auto' }}>
                      {jobLogs.map(log => (
                        <div key={log.id} style={{ marginBottom: '0.25rem', whiteSpace: 'pre-wrap' }}>
                          <span style={{ color: '#8b949e', marginRight: '0.5rem' }}>[{new Date(log.timestamp_nanos / 1_000_000).toLocaleTimeString()}]</span>
                          <span style={{ color: log.stream === 'stderr' ? '#f85149' : '#7ee787' }}>{log.content}</span>
                        </div>
                      ))}
                      {jobLogs.length === 0 && (
                        <div style={{ color: '#8b949e' }}>No console logs recorded for this job.</div>
                      )}
                    </div>
                  </div>
                </div>
              </div>
            </div>
          )}
        </div>
      )}

      {/* ---------------------------------------------------------------------- */}
      {/* TAB: Webhooks (Batch 12)                                               */}
      {/* ---------------------------------------------------------------------- */}
      {activeTab === 'webhooks' && selectedRepo && (
        <div>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '1rem' }}>
            <div>
              <h3 style={{ margin: 0 }}>🔗 Webhook Subscriptions & Event Dispatch</h3>
              <p style={{ margin: 0, fontSize: '0.85rem', color: '#8b949e' }}>
                Transactional outbox delivery with HMAC-SHA256 signatures and strict SSRF defenses
              </p>
            </div>
          </div>

          {/* New Webhook Form */}
          <div className="diff-container" style={{ marginBottom: '1.5rem' }}>
            <div className="diff-header">
              <span>Register Webhook</span>
            </div>
            <form
              onSubmit={async (e) => {
                e.preventDefault();
                if (!newWebhookUrl) return;
                try {
                  const res = await fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/webhooks`, {
                    method: 'POST',
                    headers: { ...getAuthHeaders(), 'Content-Type': 'application/json' },
                    body: JSON.stringify({
                      target_url: newWebhookUrl,
                      secret: newWebhookSecret || undefined,
                      events: newWebhookEvents.split(',').map(s => s.trim())
                    })
                  });
                  if (!res.ok) {
                    const err = await res.json();
                    alert('Error: ' + JSON.stringify(err));
                    return;
                  }
                  setNewWebhookUrl('');
                  setNewWebhookSecret('');
                  fetchWebhooks();
                } catch (e) {
                  alert('Failed to create webhook: ' + e);
                }
              }}
              style={{ padding: '1rem', display: 'flex', flexDirection: 'column', gap: '0.75rem' }}
            >
              <div>
                <label style={{ display: 'block', marginBottom: '0.25rem', fontSize: '0.85rem' }}>Target Payload URL (HTTP/HTTPS):</label>
                <input
                  type="text"
                  className="input-field"
                  placeholder="https://ci.example.com/webhook"
                  value={newWebhookUrl}
                  onChange={e => setNewWebhookUrl(e.target.value)}
                  style={{ width: '100%' }}
                  required
                />
              </div>
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: '1rem' }}>
                <div>
                  <label style={{ display: 'block', marginBottom: '0.25rem', fontSize: '0.85rem' }}>Secret (for HMAC-SHA256):</label>
                  <input
                    type="password"
                    className="input-field"
                    placeholder="Leave blank to auto-generate"
                    value={newWebhookSecret}
                    onChange={e => setNewWebhookSecret(e.target.value)}
                    style={{ width: '100%' }}
                  />
                </div>
                <div>
                  <label style={{ display: 'block', marginBottom: '0.25rem', fontSize: '0.85rem' }}>Subscribed Events (comma-separated or *):</label>
                  <input
                    type="text"
                    className="input-field"
                    value={newWebhookEvents}
                    onChange={e => setNewWebhookEvents(e.target.value)}
                    style={{ width: '100%' }}
                  />
                </div>
              </div>
              <button type="submit" className="btn btn-primary" style={{ alignSelf: 'flex-start' }}>
                + Add Webhook
              </button>
            </form>
          </div>

          {/* Subscriptions List */}
          <div className="diff-container" style={{ marginBottom: '1.5rem' }}>
            <div className="diff-header">
              <span>Active Subscriptions ({webhooks.length})</span>
            </div>
            <div style={{ padding: '0.75rem' }}>
              {webhooks.map(sub => (
                <div key={sub.id} style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', padding: '0.75rem', borderBottom: '1px solid #30363d' }}>
                  <div>
                    <strong><code>{sub.target_url}</code></strong>
                    <div style={{ fontSize: '0.8rem', color: '#8b949e', marginTop: '0.25rem' }}>
                      Events: {sub.events.join(', ')} | Status: <span style={{ color: sub.is_active ? '#3fb950' : '#f85149' }}>{sub.is_active ? 'Active' : 'Disabled'}</span>
                    </div>
                  </div>
                  <div style={{ display: 'flex', gap: '0.5rem' }}>
                    <button
                      className="btn btn-secondary"
                      onClick={() => {
                        setSelectedWebhook(sub);
                        fetchWebhookDeliveries(sub.id);
                      }}
                    >
                      📜 Deliveries
                    </button>
                    <button
                      className="btn btn-secondary"
                      style={{ color: '#f85149' }}
                      onClick={async () => {
                        await fetch(`/api/v1/webhooks/${sub.id}`, { method: 'DELETE', headers: getAuthHeaders() });
                        fetchWebhooks();
                        if (selectedWebhook?.id === sub.id) setSelectedWebhook(null);
                      }}
                    >
                      Delete
                    </button>
                  </div>
                </div>
              ))}
              {webhooks.length === 0 && (
                <div style={{ textAlign: 'center', padding: '2rem', color: '#8b949e' }}>
                  No webhook subscriptions configured for this repository.
                </div>
              )}
            </div>
          </div>

          {/* Deliveries Inspection */}
          {selectedWebhook && (
            <div className="diff-container">
              <div className="diff-header">
                <span>Recent Deliveries for <code>{selectedWebhook.target_url}</code></span>
              </div>
              <div style={{ padding: '0.75rem' }}>
                {webhookDeliveries.map(del => (
                  <div key={del.id} style={{ padding: '0.5rem', borderBottom: '1px solid #21262d', display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                    <div>
                      <span className={`badge ${del.status === 'SUCCESS' ? 'badge-open' : 'badge-closed'}`} style={{ marginRight: '0.5rem' }}>
                        {del.response_status || 'ERR'}
                      </span>
                      <strong>{del.event_type}</strong> (attempt #{del.attempt})
                    </div>
                    <div style={{ fontSize: '0.8rem', color: '#8b949e' }}>
                      {del.execution_duration_ms} ms | {new Date(del.delivered_at_nanos / 1_000_000).toLocaleTimeString()}
                    </div>
                  </div>
                ))}
                {webhookDeliveries.length === 0 && (
                  <div style={{ textAlign: 'center', padding: '1rem', color: '#8b949e' }}>
                    No delivery attempts logged yet.
                  </div>
                )}
              </div>
            </div>
          )}
        </div>
      )}

      {/* ---------------------------------------------------------------------- */}
      {/* TAB: Runners (Batch 12)                                                */}
      {/* ---------------------------------------------------------------------- */}
      {activeTab === 'runners' && (
        <div>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '1rem' }}>
            <div>
              <h3 style={{ margin: 0 }}>🏃 Self-Hosted Build Runners</h3>
              <p style={{ margin: 0, fontSize: '0.85rem', color: '#8b949e' }}>
                Isolated runner agents polling for jobs, executing within isolated sandboxes
              </p>
            </div>
          </div>

          {/* Registration Box */}
          <div className="diff-container" style={{ marginBottom: '1.5rem' }}>
            <div className="diff-header">
              <span>Register Runner</span>
            </div>
            <div style={{ padding: '1rem' }}>
              {registeredToken && (
                <div style={{ background: '#1c2128', border: '1px solid #3fb950', padding: '1rem', borderRadius: '6px', marginBottom: '1rem' }}>
                  <strong style={{ color: '#3fb950' }}>Runner Registered Successfully!</strong>
                  <p style={{ fontSize: '0.85rem', margin: '0.5rem 0' }}>Save this authentication token immediately; it will not be shown again:</p>
                  <code style={{ fontSize: '0.95rem', background: '#0d1117', padding: '0.5rem', display: 'block', borderRadius: '4px' }}>
                    {registeredToken}
                  </code>
                </div>
              )}
              <form
                onSubmit={async (e) => {
                  e.preventDefault();
                  if (!newRunnerName) return;
                  try {
                    const res = await fetch('/api/v1/runners/register', {
                      method: 'POST',
                      headers: { ...getAuthHeaders(), 'Content-Type': 'application/json' },
                      body: JSON.stringify({
                        name: newRunnerName,
                        hostname: 'localhost',
                        os: 'windows',
                        arch: 'x86_64',
                        labels: newRunnerLabels.split(',').map(s => s.trim()),
                        max_concurrency: 2
                      })
                    });
                    if (res.ok) {
                      const data = await res.json();
                      setRegisteredToken(data.token);
                      setNewRunnerName('');
                      fetchRunners();
                    } else {
                      const err = await res.json();
                      alert('Registration failed: ' + JSON.stringify(err));
                    }
                  } catch (e) {
                    alert('Error: ' + e);
                  }
                }}
                style={{ display: 'flex', gap: '1rem', alignItems: 'flex-end' }}
              >
                <div style={{ flex: 1 }}>
                  <label style={{ display: 'block', fontSize: '0.85rem', marginBottom: '0.25rem' }}>Runner Name:</label>
                  <input
                    type="text"
                    className="input-field"
                    placeholder="windows-build-worker-01"
                    value={newRunnerName}
                    onChange={e => setNewRunnerName(e.target.value)}
                    style={{ width: '100%' }}
                    required
                  />
                </div>
                <div style={{ flex: 1 }}>
                  <label style={{ display: 'block', fontSize: '0.85rem', marginBottom: '0.25rem' }}>Labels (comma-separated):</label>
                  <input
                    type="text"
                    className="input-field"
                    value={newRunnerLabels}
                    onChange={e => setNewRunnerLabels(e.target.value)}
                    style={{ width: '100%' }}
                  />
                </div>
                <button type="submit" className="btn btn-primary">
                  + Register Runner
                </button>
              </form>
            </div>
          </div>

          {/* Runners List */}
          <div className="diff-container">
            <div className="diff-header">
              <span>Connected Runners ({runners.length})</span>
            </div>
            <div style={{ padding: '0.75rem' }}>
              {runners.map(r => (
                <div key={r.id} style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', padding: '0.75rem', borderBottom: '1px solid #30363d' }}>
                  <div>
                    <div style={{ display: 'flex', alignItems: 'center', gap: '0.5rem' }}>
                      <strong>{r.name}</strong>
                      <span className={`badge ${r.status === 'ACTIVE' ? 'badge-open' : 'badge-closed'}`}>
                        {r.status}
                      </span>
                    </div>
                    <div style={{ fontSize: '0.8rem', color: '#8b949e', marginTop: '0.25rem' }}>
                      Platform: <code>{r.os}/{r.arch}</code> | Host: <code>{r.hostname}</code> | Labels: <code>{r.labels.join(', ')}</code>
                    </div>
                  </div>
                  <div style={{ textAlign: 'right', fontSize: '0.8rem', color: '#8b949e' }}>
                    <div>Concurrency: {r.active_jobs} / {r.max_concurrency}</div>
                    <div>Heartbeat: {new Date(r.last_heartbeat_at_nanos / 1_000_000).toLocaleTimeString()}</div>
                  </div>
                </div>
              ))}
              {runners.length === 0 && (
                <div style={{ textAlign: 'center', padding: '2rem', color: '#8b949e' }}>
                  No runners registered yet. Register a runner to start processing jobs.
                </div>
              )}
            </div>
          </div>
        </div>
      )}

      {/* ---------------------------------------------------------------------- */}
      {/* TAB: Secrets (Batch 12)                                                */}
      {/* ---------------------------------------------------------------------- */}
      {activeTab === 'secrets' && selectedRepo && (
        <div>
          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '1rem' }}>
            <div>
              <h3 style={{ margin: 0 }}>🔑 Repository Secrets</h3>
              <p style={{ margin: 0, fontSize: '0.85rem', color: '#8b949e' }}>
                Encrypted secrets injected securely into isolated runner environments and masked in logs
              </p>
            </div>
          </div>

          {/* Add Secret Form */}
          <div className="diff-container" style={{ marginBottom: '1.5rem' }}>
            <div className="diff-header">
              <span>Add / Update Secret</span>
            </div>
            <form
              onSubmit={async (e) => {
                e.preventDefault();
                if (!newSecretName || !newSecretValue) return;
                try {
                  await fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/secrets/${newSecretName}`, {
                    method: 'PUT',
                    headers: { ...getAuthHeaders(), 'Content-Type': 'application/json' },
                    body: JSON.stringify({ value: newSecretValue })
                  });
                  setNewSecretName('');
                  setNewSecretValue('');
                  fetchSecrets();
                } catch (e) {
                  alert('Failed to save secret: ' + e);
                }
              }}
              style={{ padding: '1rem', display: 'flex', gap: '1rem', alignItems: 'flex-end' }}
            >
              <div style={{ flex: 1 }}>
                <label style={{ display: 'block', fontSize: '0.85rem', marginBottom: '0.25rem' }}>Secret Name (uppercase):</label>
                <input
                  type="text"
                  className="input-field"
                  placeholder="DEPLOY_KEY"
                  value={newSecretName}
                  onChange={e => setNewSecretName(e.target.value)}
                  style={{ width: '100%' }}
                  required
                />
              </div>
              <div style={{ flex: 2 }}>
                <label style={{ display: 'block', fontSize: '0.85rem', marginBottom: '0.25rem' }}>Secret Value:</label>
                <input
                  type="password"
                  className="input-field"
                  placeholder="Enter secret ciphertext / token"
                  value={newSecretValue}
                  onChange={e => setNewSecretValue(e.target.value)}
                  style={{ width: '100%' }}
                  required
                />
              </div>
              <button type="submit" className="btn btn-primary">
                Save Secret
              </button>
            </form>
          </div>

          {/* Secrets List */}
          <div className="diff-container">
            <div className="diff-header">
              <span>Stored Secrets ({secrets.length})</span>
            </div>
            <div style={{ padding: '0.75rem' }}>
              {secrets.map(s => (
                <div key={s.id} style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', padding: '0.75rem', borderBottom: '1px solid #30363d' }}>
                  <div>
                    <strong><code>{s.name}</code></strong>
                    <div style={{ fontSize: '0.8rem', color: '#8b949e', marginTop: '0.25rem' }}>
                      Updated: {new Date(s.updated_at_nanos / 1_000_000).toLocaleString()}
                    </div>
                  </div>
                  <button
                    className="btn btn-secondary"
                    style={{ color: '#f85149' }}
                    onClick={async () => {
                      await fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/secrets/${s.name}`, {
                        method: 'DELETE',
                        headers: getAuthHeaders()
                      });
                      fetchSecrets();
                    }}
                  >
                    Delete
                  </button>
                </div>
              ))}
              {secrets.length === 0 && (
                <div style={{ textAlign: 'center', padding: '2rem', color: '#8b949e' }}>
                  No secrets defined. Secrets defined here are available as environment variables in workflow runners.
                </div>
              )}
            </div>
          </div>
        </div>
      )}
      {showNotificationsModal && (
        <div className="modal-backdrop" onClick={() => setShowNotificationsModal(false)}>
          <div className="modal-box" style={{ width: '560px' }} onClick={(e) => e.stopPropagation()}>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '1rem' }}>
              <h3 style={{ margin: 0 }}>Notifications</h3>
              <div style={{ display: 'flex', gap: '0.5rem' }}>
                <button
                  className="btn btn-secondary"
                  style={{ fontSize: '0.75rem' }}
                  onClick={handleMarkAllNotificationsRead}
                >
                  Mark all as read
                </button>
                <button
                  className="btn btn-secondary"
                  style={{ fontSize: '0.75rem' }}
                  onClick={() => setShowNotificationsModal(false)}
                >
                  Close
                </button>
              </div>
            </div>

            <div>
              {notifications.map((n) => (
                <div
                  key={n.id}
                  style={{
                    padding: '0.75rem',
                    borderBottom: '1px solid #30363d',
                    backgroundColor: n.is_read ? 'transparent' : 'rgba(31, 111, 235, 0.1)',
                  }}
                >
                  <div style={{ fontWeight: 600, fontSize: '0.9rem', color: '#e6edf3' }}>{n.title}</div>
                  <div style={{ fontSize: '0.8rem', color: '#8b949e', margin: '0.2rem 0' }}>{n.message}</div>
                  <div style={{ fontSize: '0.75rem', color: '#6e7681' }}>
                    {new Date(n.created_at_nanos / 1_000_000).toLocaleString()}
                  </div>
                </div>
              ))}
              {notifications.length === 0 && (
                <div style={{ padding: '2rem', textAlign: 'center', color: '#8b949e' }}>
                  No unread notifications.
                </div>
              )}
            </div>
          </div>
        </div>
      )}

      {/* ---------------------------------------------------------------------- */}
      {/* MODAL: New Issue                                                       */}
      {/* ---------------------------------------------------------------------- */}
      {showNewIssueModal && (
        <div className="modal-backdrop" onClick={() => setShowNewIssueModal(false)}>
          <div className="modal-box" onClick={(e) => e.stopPropagation()}>
            <h3>Create New Issue</h3>
            <form onSubmit={handleCreateIssue}>
              <div className="form-group">
                <label className="form-label">Title</label>
                <input
                  type="text"
                  required
                  className="form-input"
                  placeholder="Issue title"
                  value={newIssueForm.title}
                  onChange={(e) => setNewIssueForm({ ...newIssueForm, title: e.target.value })}
                />
              </div>
              <div className="form-group">
                <label className="form-label">Description</label>
                <textarea
                  className="form-input"
                  rows={4}
                  placeholder="Describe the issue..."
                  value={newIssueForm.body}
                  onChange={(e) => setNewIssueForm({ ...newIssueForm, body: e.target.value })}
                />
              </div>
              <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '0.5rem', marginTop: '1rem' }}>
                <button type="button" className="btn btn-secondary" onClick={() => setShowNewIssueModal(false)}>
                  Cancel
                </button>
                <button type="submit" className="btn btn-primary">
                  Submit Issue
                </button>
              </div>
            </form>
          </div>
        </div>
      )}

      {/* ---------------------------------------------------------------------- */}
      {/* MODAL: New Pull Request                                                */}
      {/* ---------------------------------------------------------------------- */}
      {showNewPrModal && (
        <div className="modal-backdrop" onClick={() => setShowNewPrModal(false)}>
          <div className="modal-box" style={{ width: '560px' }} onClick={(e) => e.stopPropagation()}>
            <h3>Open a Pull Request</h3>
            <form onSubmit={handleCreatePullRequest}>
              <div style={{ display: 'flex', gap: '1rem', marginBottom: '1rem' }}>
                <div style={{ flex: 1 }}>
                  <label className="form-label">Base branch</label>
                  <select
                    className="form-input"
                    value={newPrForm.base_branch}
                    onChange={(e) => setNewPrForm({ ...newPrForm, base_branch: e.target.value })}
                  >
                    {branches.map((b) => (
                      <option key={b.name} value={b.name}>
                        {b.name}
                      </option>
                    ))}
                  </select>
                </div>
                <div style={{ flex: 1 }}>
                  <label className="form-label">Compare branch</label>
                  <select
                    className="form-input"
                    value={newPrForm.head_branch}
                    onChange={(e) => setNewPrForm({ ...newPrForm, head_branch: e.target.value })}
                  >
                    {branches.map((b) => (
                      <option key={b.name} value={b.name}>
                        {b.name}
                      </option>
                    ))}
                  </select>
                </div>
              </div>

              <div className="form-group">
                <label className="form-label">Title</label>
                <input
                  type="text"
                  required
                  className="form-input"
                  placeholder="Pull request title"
                  value={newPrForm.title}
                  onChange={(e) => setNewPrForm({ ...newPrForm, title: e.target.value })}
                />
              </div>

              <div className="form-group">
                <label className="form-label">Description (optional)</label>
                <textarea
                  className="form-input"
                  rows={4}
                  placeholder="Describe your changes..."
                  value={newPrForm.body}
                  onChange={(e) => setNewPrForm({ ...newPrForm, body: e.target.value })}
                />
              </div>

              <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '0.5rem', marginTop: '1rem' }}>
                <button type="button" className="btn btn-secondary" onClick={() => setShowNewPrModal(false)}>
                  Cancel
                </button>
                <button type="submit" className="btn btn-primary">
                  Create Pull Request
                </button>
              </div>
            </form>
          </div>
        </div>
      )}

      {/* ---------------------------------------------------------------------- */}
      {/* MODAL: New Organization                                                */}
      {/* ---------------------------------------------------------------------- */}
      {showNewOrgModal && (
        <div className="modal-backdrop" onClick={() => setShowNewOrgModal(false)}>
          <div className="modal-box" onClick={(e) => e.stopPropagation()}>
            <h3>Create an Organization</h3>
            <form onSubmit={handleCreateOrg}>
              <div className="form-group">
                <label className="form-label">Organization Name (Slug)</label>
                <input
                  type="text"
                  required
                  className="form-input"
                  placeholder="e.g. acme-corp"
                  value={newOrgForm.name}
                  onChange={(e) => setNewOrgForm({ ...newOrgForm, name: e.target.value })}
                />
              </div>
              <div className="form-group">
                <label className="form-label">Display Name</label>
                <input
                  type="text"
                  className="form-input"
                  placeholder="e.g. Acme Corporation"
                  value={newOrgForm.display_name}
                  onChange={(e) => setNewOrgForm({ ...newOrgForm, display_name: e.target.value })}
                />
              </div>
              <div className="form-group">
                <label className="form-label">Description</label>
                <textarea
                  className="form-input"
                  rows={3}
                  placeholder="Organization description..."
                  value={newOrgForm.description}
                  onChange={(e) => setNewOrgForm({ ...newOrgForm, description: e.target.value })}
                />
              </div>
              <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '0.5rem', marginTop: '1rem' }}>
                <button type="button" className="btn btn-secondary" onClick={() => setShowNewOrgModal(false)}>
                  Cancel
                </button>
                <button type="submit" className="btn btn-primary">
                  Create Organization
                </button>
              </div>
            </form>
          </div>
        </div>
      )}

      {/* ---------------------------------------------------------------------- */}
      {/* MODAL: New Team                                                        */}
      {/* ---------------------------------------------------------------------- */}
      {showNewTeamModal && selectedOrg && (
        <div className="modal-backdrop" onClick={() => setShowNewTeamModal(false)}>
          <div className="modal-box" onClick={(e) => e.stopPropagation()}>
            <h3>Add Team to {selectedOrg.display_name}</h3>
            <form onSubmit={handleCreateTeam}>
              <div className="form-group">
                <label className="form-label">Team Name</label>
                <input
                  type="text"
                  required
                  className="form-input"
                  placeholder="e.g. Core Engineering"
                  value={newTeamForm.name}
                  onChange={(e) => setNewTeamForm({ ...newTeamForm, name: e.target.value })}
                />
              </div>
              <div className="form-group">
                <label className="form-label">Description (optional)</label>
                <textarea
                  className="form-input"
                  rows={3}
                  placeholder="Team description..."
                  value={newTeamForm.description}
                  onChange={(e) => setNewTeamForm({ ...newTeamForm, description: e.target.value })}
                />
              </div>
              <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '0.5rem', marginTop: '1rem' }}>
                <button type="button" className="btn btn-secondary" onClick={() => setShowNewTeamModal(false)}>
                  Cancel
                </button>
                <button type="submit" className="btn btn-primary">
                  Create Team
                </button>
              </div>
            </form>
          </div>
        </div>
      )}

      {/* ---------------------------------------------------------------------- */}
      {/* MODAL: New Branch Protection Rule                                      */}
      {/* ---------------------------------------------------------------------- */}
      {showNewRuleModal && (
        <div className="modal-backdrop" onClick={() => setShowNewRuleModal(false)}>
          <div className="modal-box" onClick={(e) => e.stopPropagation()}>
            <h3>Protect a Branch</h3>
            <form onSubmit={handleCreateProtectionRule}>
              <div className="form-group">
                <label className="form-label">Branch pattern</label>
                <input
                  type="text"
                  required
                  className="form-input"
                  placeholder="e.g. main or release/*"
                  value={newRuleForm.branch_pattern}
                  onChange={(e) => setNewRuleForm({ ...newRuleForm, branch_pattern: e.target.value })}
                />
              </div>
              <div className="form-group">
                <label className="form-label">Required approvals</label>
                <input
                  type="number"
                  min={1}
                  max={5}
                  className="form-input"
                  value={newRuleForm.required_approvals}
                  onChange={(e) => setNewRuleForm({ ...newRuleForm, required_approvals: parseInt(e.target.value) || 1 })}
                />
              </div>
              <div className="form-group">
                <label>
                  <input
                    type="checkbox"
                    checked={newRuleForm.prevent_self_approval}
                    onChange={(e) => setNewRuleForm({ ...newRuleForm, prevent_self_approval: e.target.checked })}
                  />{' '}
                  Prevent self-approval by author
                </label>
              </div>
              <div className="form-group">
                <label>
                  <input
                    type="checkbox"
                    checked={newRuleForm.require_changes_addressed}
                    onChange={(e) => setNewRuleForm({ ...newRuleForm, require_changes_addressed: e.target.checked })}
                  />{' '}
                  Require resolution of inline review comments
                </label>
              </div>
              <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '0.5rem', marginTop: '1rem' }}>
                <button type="button" className="btn btn-secondary" onClick={() => setShowNewRuleModal(false)}>
                  Cancel
                </button>
                <button type="submit" className="btn btn-primary">
                  Save Rule
                </button>
              </div>
            </form>
          </div>
        </div>
      )}

      {/* ---------------------------------------------------------------------- */}
      {/* MODAL: New Repository                                                  */}
      {/* ---------------------------------------------------------------------- */}
      {showNewRepoModal && (
        <div className="modal-backdrop" onClick={() => setShowNewRepoModal(false)}>
          <div className="modal-box" onClick={(e) => e.stopPropagation()}>
            <h3>Create a New Repository</h3>
            <form
              onSubmit={async (e) => {
                e.preventDefault();
                try {
                  const res = await fetch('/api/v1/repos', {
                    method: 'POST',
                    headers: { ...getAuthHeaders(), 'Content-Type': 'application/json' },
                    body: JSON.stringify(newRepoForm),
                  });
                  if (!res.ok) throw new Error('Failed to create repository');
                  setShowNewRepoModal(false);
                  fetchRepos();
                } catch (err: any) {
                  alert(err.message);
                }
              }}
            >
              <div className="form-group">
                <label className="form-label">Repository Name</label>
                <input
                  type="text"
                  required
                  className="form-input"
                  value={newRepoForm.name}
                  onChange={(e) => setNewRepoForm({ ...newRepoForm, name: e.target.value })}
                />
              </div>
              <div className="form-group">
                <label className="form-label">Description (optional)</label>
                <textarea
                  className="form-input"
                  rows={3}
                  value={newRepoForm.description}
                  onChange={(e) => setNewRepoForm({ ...newRepoForm, description: e.target.value })}
                />
              </div>
              <div className="form-group">
                <label>
                  <input
                    type="checkbox"
                    checked={newRepoForm.is_private}
                    onChange={(e) => setNewRepoForm({ ...newRepoForm, is_private: e.target.checked })}
                  />{' '}
                  Private repository
                </label>
              </div>
              <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '0.5rem', marginTop: '1rem' }}>
                <button type="button" className="btn btn-secondary" onClick={() => setShowNewRepoModal(false)}>
                  Cancel
                </button>
                <button type="submit" className="btn btn-primary">
                  Create Repository
                </button>
              </div>
            </form>
          </div>
        </div>
      )}

      {/* ---------------------------------------------------------------------- */}
      {/* MODAL: Sign in / Sign up                                               */}
      {/* ---------------------------------------------------------------------- */}
      {showAuthModal && (
        <div className="modal-backdrop" onClick={() => setShowAuthModal(false)}>
          <div className="modal-box" onClick={(e) => e.stopPropagation()}>
            <h3>{authMode === 'login' ? 'Sign in to Spine Forge' : 'Create your account'}</h3>
            {authError && <div style={{ color: '#f85149', marginBottom: '1rem', fontSize: '0.85rem' }}>{authError}</div>}
            <form onSubmit={handleAuthSubmit}>
              <div className="form-group">
                <label className="form-label">Username</label>
                <input
                  type="text"
                  required
                  className="form-input"
                  value={authForm.username}
                  onChange={(e) => setAuthForm({ ...authForm, username: e.target.value })}
                />
              </div>
              {authMode === 'register' && (
                <div className="form-group">
                  <label className="form-label">Email</label>
                  <input
                    type="email"
                    required
                    className="form-input"
                    value={authForm.email}
                    onChange={(e) => setAuthForm({ ...authForm, email: e.target.value })}
                  />
                </div>
              )}
              <div className="form-group">
                <label className="form-label">Password</label>
                <input
                  type="password"
                  required
                  className="form-input"
                  value={authForm.password}
                  onChange={(e) => setAuthForm({ ...authForm, password: e.target.value })}
                />
              </div>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginTop: '1rem' }}>
                <span
                  style={{ fontSize: '0.85rem', color: '#58a6ff', cursor: 'pointer' }}
                  onClick={() => setAuthMode(authMode === 'login' ? 'register' : 'login')}
                >
                  {authMode === 'login' ? 'Need an account? Sign up' : 'Already have an account? Sign in'}
                </span>
                <button type="submit" className="btn btn-primary">
                  {authMode === 'login' ? 'Sign in' : 'Create account'}
                </button>
              </div>
            </form>
          </div>
        </div>
      )}
      {/* ---------------------------------------------------------------------- */}
      {/* MODAL: New Release (Batch 11)                                          */}
      {/* ---------------------------------------------------------------------- */}
      {showNewReleaseModal && selectedRepo && (
        <div className="modal-backdrop" onClick={() => setShowNewReleaseModal(false)}>
          <div className="modal-box" onClick={(e) => e.stopPropagation()}>
            <h3 style={{ margin: '0 0 1rem 0' }}>Draft a new release for {selectedRepo.name}</h3>
            <form onSubmit={handleCreateRelease}>
              <div className="form-group">
                <label className="form-label">Tag version *</label>
                <input
                  type="text"
                  className="form-input"
                  placeholder="e.g. v1.0.0"
                  value={newReleaseForm.tag_name}
                  onChange={(e) => setNewReleaseForm({ ...newReleaseForm, tag_name: e.target.value })}
                  required
                />
              </div>
              <div className="form-group">
                <label className="form-label">Release title *</label>
                <input
                  type="text"
                  className="form-input"
                  placeholder="Release title"
                  value={newReleaseForm.title}
                  onChange={(e) => setNewReleaseForm({ ...newReleaseForm, title: e.target.value })}
                  required
                />
              </div>
              <div className="form-group">
                <label className="form-label">Release notes</label>
                <textarea
                  className="form-input"
                  rows={4}
                  placeholder="Describe this release..."
                  value={newReleaseForm.body}
                  onChange={(e) => setNewReleaseForm({ ...newReleaseForm, body: e.target.value })}
                />
              </div>
              <div style={{ display: 'flex', gap: '1rem', marginBottom: '1rem' }}>
                <label style={{ display: 'flex', alignItems: 'center', gap: '0.4rem', fontSize: '0.85rem' }}>
                  <input
                    type="checkbox"
                    checked={newReleaseForm.is_prerelease}
                    onChange={(e) => setNewReleaseForm({ ...newReleaseForm, is_prerelease: e.target.checked })}
                  />
                  Pre-release
                </label>
                <label style={{ display: 'flex', alignItems: 'center', gap: '0.4rem', fontSize: '0.85rem' }}>
                  <input
                    type="checkbox"
                    checked={newReleaseForm.is_draft}
                    onChange={(e) => setNewReleaseForm({ ...newReleaseForm, is_draft: e.target.checked })}
                  />
                  Draft
                </label>
              </div>
              <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '0.5rem' }}>
                <button type="button" className="btn btn-secondary" onClick={() => setShowNewReleaseModal(false)}>
                  Cancel
                </button>
                <button type="submit" className="btn btn-primary">
                  Publish release
                </button>
              </div>
            </form>
          </div>
        </div>
      )}
    </div>
  );
};
export default App;
