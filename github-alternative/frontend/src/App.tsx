import React, { useEffect, useState } from 'react';
import {
  RepositoryDto,
  CommitDto,
  BranchDto,
  TreeEntryDto,
  BlobContentDto,
  UserDto,
  SessionResponse,
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
  const [activeTab, setActiveTab] = useState<'files' | 'commits'>('files');
  const [treeEntries, setTreeEntries] = useState<TreeEntryDto[]>([]);
  const [selectedBlob, setSelectedBlob] = useState<BlobContentDto | null>(null);
  const [currentPath, setCurrentPath] = useState<string>('');
  const [loading, setLoading] = useState<boolean>(true);

  // New Repository Modal
  const [showNewRepoModal, setShowNewRepoModal] = useState<boolean>(false);
  const [newRepoForm, setNewRepoForm] = useState({
    name: '',
    description: '',
    is_private: false,
    durability: 'ThreeCopies',
  });
  const [createRepoError, setCreateRepoError] = useState<string | null>(null);

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
      .then((res) => {
        if (!res.ok) throw new Error(`HTTP error! status: ${res.status}`);
        return res.json();
      })
      .then((data: RepositoryDto[]) => {
        setRepos(data);
        if (data.length > 0 && !selectedRepo) {
          setSelectedRepo(data[0]);
        }
        setLoading(false);
      })
      .catch(() => {
        // Fallback demo repos when standalone
        const demo: RepositoryDto[] = [
          {
            id: 'repo_01',
            owner: 'alice',
            name: 'spine-core',
            description: 'High-performance Rust-native distributed version-control engine',
            is_private: false,
            default_branch: 'main',
            native_repo_id: 'alice_spine-core',
            durability_policy: 'ThreeCopies',
            provisioning_status: 'READY',
            created_at_nanos: Date.now() * 1_000_000,
            updated_at_nanos: Date.now() * 1_000_000,
          },
          {
            id: 'repo_02',
            owner: 'alice',
            name: 'spine-forge',
            description: 'Modern developer collaboration platform and native storage nodes',
            is_private: true,
            default_branch: 'main',
            native_repo_id: 'alice_spine-forge',
            durability_policy: 'ThreeCopies',
            provisioning_status: 'READY',
            created_at_nanos: Date.now() * 1_000_000,
            updated_at_nanos: Date.now() * 1_000_000,
          },
        ];
        setRepos(demo);
        if (!selectedRepo) setSelectedRepo(demo[0]);
        setLoading(false);
      });
  };

  useEffect(() => {
    fetchRepos();
  }, [currentUser]);

  // Load branches, commits, and tree when selected repository changes
  useEffect(() => {
    if (!selectedRepo) return;
    setCurrentBranch(selectedRepo.default_branch || 'main');
    setSelectedBlob(null);
    setCurrentPath('');

    // Fetch branches
    fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/branches`, {
      headers: getAuthHeaders(),
    })
      .then((res) => (res.ok ? res.json() : []))
      .then((data: BranchDto[]) => {
        setBranches(data);
      })
      .catch(() => setBranches([{ name: 'main', commit_id: '4f2910...' }]));

    // Fetch commits
    fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/commits?ref=${selectedRepo.default_branch || 'main'}`, {
      headers: getAuthHeaders(),
    })
      .then((res) => (res.ok ? res.json() : []))
      .then((data: CommitDto[]) => {
        setCommits(data);
      })
      .catch(() => {
        setCommits([
          {
            id: 'e4d909c290d0fb1ca068ffaddf22cbd0add22cbd0add22cbd0add22cbd0add22',
            parents: [],
            author_name: 'Alice Engineer',
            author_email: 'alice@spine.build',
            timestamp_nanos: 1772580000_000_000_000,
            message: 'feat: implement initial content-addressed storage engine',
          },
        ]);
      });

    // Fetch root tree
    fetchTree(selectedRepo.owner, selectedRepo.name, selectedRepo.default_branch || 'main', '');
  }, [selectedRepo]);

  const fetchTree = (owner: string, name: string, ref: string, path: string) => {
    const url = `/api/v1/repos/${owner}/${name}/tree?ref=${ref}${path ? `&path=${encodeURIComponent(path)}` : ''}`;
    fetch(url, { headers: getAuthHeaders() })
      .then((res) => (res.ok ? res.json() : []))
      .then((entries: TreeEntryDto[]) => {
        setTreeEntries(entries);
      })
      .catch(() => {
        setTreeEntries([
          { name: 'src', path: 'src', is_dir: true, object_id: 'd19...', mode: 0o040000 },
          { name: 'Cargo.toml', path: 'Cargo.toml', is_dir: false, object_id: 'f82...', mode: 0o100644 },
          { name: 'README.md', path: 'README.md', is_dir: false, object_id: 'a90...', mode: 0o100644 },
        ]);
      });
  };

  const handleFileClick = (entry: TreeEntryDto) => {
    if (!selectedRepo) return;
    if (entry.is_dir) {
      setCurrentPath(entry.path);
      fetchTree(selectedRepo.owner, selectedRepo.name, currentBranch, entry.path);
    } else {
      // Fetch blob
      const url = `/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/blob/${encodeURIComponent(entry.path)}?ref=${currentBranch}`;
      fetch(url, { headers: getAuthHeaders() })
        .then((res) => (res.ok ? res.json() : null))
        .then((blob: BlobContentDto | null) => {
          if (blob) setSelectedBlob(blob);
        })
        .catch(() => {
          setSelectedBlob({
            path: entry.path,
            size: 120,
            is_binary: false,
            content: '# ' + entry.name + '\n\nPreview of native Spine file.',
            object_id: entry.object_id,
          });
        });
    }
  };

  // Auth actions
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
      if (!res.ok) {
        throw new Error(data.message || data.error || 'Authentication failed');
      }

      const token = data.token;
      if (token) {
        localStorage.setItem('spine_token', token);
        setAuthToken(token);
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
        await fetch('/api/v1/auth/logout', {
          method: 'POST',
          headers: getAuthHeaders(),
        });
      } catch (_) {}
    }
    localStorage.removeItem('spine_token');
    setAuthToken('');
    setCurrentUser(null);
    fetchRepos();
  };

  // Create repository action
  const handleCreateRepo = async (e: React.FormEvent) => {
    e.preventDefault();
    setCreateRepoError(null);
    try {
      const res = await fetch('/api/v1/repos', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          ...getAuthHeaders(),
        },
        body: JSON.stringify(newRepoForm),
      });
      const data = await res.json();
      if (!res.ok) {
        throw new Error(data.message || data.error || 'Failed to create repository');
      }
      setShowNewRepoModal(false);
      setNewRepoForm({ name: '', description: '', is_private: false, durability: 'ThreeCopies' });
      fetchRepos();
    } catch (err: any) {
      setCreateRepoError(err.message || 'Failed to create repository');
    }
  };

  return (
    <div className="container">
      {/* Top Header */}
      <header className="header">
        <div className="logo" onClick={() => setSelectedBlob(null)}>
          <span>SPINE FORGE</span>
          <span className="badge">v0.1.0-alpha</span>
        </div>
        <div className="nav-actions">
          {currentUser ? (
            <>
              <span style={{ fontSize: '0.9rem', color: '#c9d1d9' }}>
                Signed in as <strong>{currentUser.username}</strong>
              </span>
              <button
                className="btn btn-primary"
                onClick={() => setShowNewRepoModal(true)}
              >
                + New Repository
              </button>
              <button className="btn btn-secondary" onClick={handleLogout}>
                Sign Out
              </button>
            </>
          ) : (
            <>
              <button
                className="btn btn-secondary"
                onClick={() => {
                  setAuthMode('login');
                  setShowAuthModal(true);
                }}
              >
                Sign In
              </button>
              <button
                className="btn btn-primary"
                onClick={() => {
                  setAuthMode('register');
                  setShowAuthModal(true);
                }}
              >
                Register
              </button>
            </>
          )}
        </div>
      </header>

      {/* Main Grid: Left Repositories List, Right Selected Repo View */}
      {loading ? (
        <p style={{ color: '#8b949e' }}>Loading Spine Forge repositories...</p>
      ) : (
        <div style={{ display: 'grid', gridTemplateColumns: '320px 1fr', gap: '2rem' }}>
          {/* Repositories Column */}
          <div>
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '1rem' }}>
              <h2 style={{ fontSize: '1.1rem', margin: 0 }}>Repositories</h2>
              {currentUser && (
                <button
                  className="btn btn-secondary"
                  style={{ fontSize: '0.8rem', padding: '0.25rem 0.5rem' }}
                  onClick={() => setShowNewRepoModal(true)}
                >
                  New
                </button>
              )}
            </div>

            {repos.length === 0 ? (
              <p style={{ color: '#8b949e', fontSize: '0.9rem' }}>No repositories available.</p>
            ) : (
              repos.map((repo) => (
                <div
                  key={repo.id}
                  className={`repo-card ${selectedRepo?.id === repo.id ? 'repo-card-selected' : ''}`}
                  onClick={() => {
                    setSelectedRepo(repo);
                    setSelectedBlob(null);
                  }}
                >
                  <div className="repo-title">
                    <span>{repo.owner}/{repo.name}</span>
                  </div>
                  <div className="repo-desc">{repo.description || 'No description provided.'}</div>
                  <div className="repo-meta">
                    <span className={`badge ${repo.is_private ? 'badge-private' : 'badge-ready'}`}>
                      {repo.is_private ? 'Private' : 'Public'}
                    </span>
                    <span className="badge badge-durability">{repo.durability_policy}</span>
                    <span
                      className={`badge ${
                        repo.provisioning_status === 'READY'
                          ? 'badge-ready'
                          : repo.provisioning_status === 'PROVISIONING'
                          ? 'badge-provisioning'
                          : 'badge-pending'
                      }`}
                    >
                      {repo.provisioning_status}
                    </span>
                  </div>
                </div>
              ))
            )}
          </div>

          {/* Active Repository Main Panel */}
          <div>
            {selectedRepo ? (
              <div>
                {/* Repository Header */}
                <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', marginBottom: '1rem' }}>
                  <div>
                    <h1 style={{ fontSize: '1.4rem', margin: '0 0 0.5rem 0', display: 'flex', alignItems: 'center', gap: '0.5rem' }}>
                      <span style={{ color: '#8b949e' }}>{selectedRepo.owner}</span>
                      <span style={{ color: '#8b949e' }}>/</span>
                      <span style={{ color: '#58a6ff' }}>{selectedRepo.name}</span>
                    </h1>
                    <p style={{ color: '#8b949e', margin: 0, fontSize: '0.95rem' }}>
                      {selectedRepo.description || 'No description provided.'}
                    </p>
                  </div>
                  <div style={{ display: 'flex', gap: '0.5rem' }}>
                    <span className={`badge ${selectedRepo.is_private ? 'badge-private' : 'badge-ready'}`}>
                      {selectedRepo.is_private ? 'Private' : 'Public'}
                    </span>
                    <span className="badge badge-durability">{selectedRepo.durability_policy}</span>
                  </div>
                </div>

                {/* Clone Command Box */}
                <div className="clone-box">
                  <div>
                    <span style={{ color: '#8b949e', fontSize: '0.8rem', display: 'block', marginBottom: '0.2rem' }}>
                      Native Spine Clone URL:
                    </span>
                    <span className="clone-cmd">
                      spine clone spine://127.0.0.1:4434/{selectedRepo.owner}/{selectedRepo.name}
                    </span>
                  </div>
                  <button
                    className="btn btn-secondary"
                    style={{ fontSize: '0.8rem' }}
                    onClick={() => {
                      navigator.clipboard.writeText(
                        `spine clone spine://127.0.0.1:4434/${selectedRepo.owner}/${selectedRepo.name}`
                      );
                    }}
                  >
                    Copy
                  </button>
                </div>

                {/* Branch Switcher & Tabs */}
                <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: '1rem' }}>
                  <div className="tabs" style={{ margin: 0, border: 'none' }}>
                    <div
                      className={`tab ${activeTab === 'files' ? 'tab-active' : ''}`}
                      onClick={() => {
                        setActiveTab('files');
                        setSelectedBlob(null);
                      }}
                    >
                      Files
                    </div>
                    <div
                      className={`tab ${activeTab === 'commits' ? 'tab-active' : ''}`}
                      onClick={() => setActiveTab('commits')}
                    >
                      Commits ({commits.length})
                    </div>
                  </div>

                  <div style={{ display: 'flex', alignItems: 'center', gap: '0.5rem' }}>
                    <span style={{ fontSize: '0.85rem', color: '#8b949e' }}>Branch:</span>
                    <select
                      className="form-input"
                      style={{ padding: '0.25rem 0.5rem', width: 'auto' }}
                      value={currentBranch}
                      onChange={(e) => setCurrentBranch(e.target.value)}
                    >
                      {branches.length > 0 ? (
                        branches.map((b) => (
                          <option key={b.name} value={b.name}>
                            {b.name}
                          </option>
                        ))
                      ) : (
                        <option value="main">main</option>
                      )}
                    </select>
                  </div>
                </div>

                {/* Tab: Files View */}
                {activeTab === 'files' && (
                  <div>
                    {selectedBlob ? (
                      /* File Viewer */
                      <div className="file-viewer">
                        <div className="file-viewer-header">
                          <div>
                            <span
                              style={{ color: '#58a6ff', cursor: 'pointer', marginRight: '0.5rem' }}
                              onClick={() => setSelectedBlob(null)}
                            >
                              &larr; Back to tree
                            </span>
                            <strong>{selectedBlob.path}</strong> ({selectedBlob.size} bytes)
                          </div>
                          <span>SHA: {selectedBlob.object_id.substring(0, 10)}</span>
                        </div>
                        {selectedBlob.is_binary ? (
                          <div style={{ padding: '2rem', textAlign: 'center', color: '#8b949e' }}>
                            Binary file ({selectedBlob.size} bytes). Preview not available in browser.
                          </div>
                        ) : (
                          <pre className="file-content">{selectedBlob.content}</pre>
                        )}
                      </div>
                    ) : (
                      /* Tree Table */
                      <div>
                        {currentPath && (
                          <div style={{ marginBottom: '0.5rem' }}>
                            <button
                              className="btn btn-secondary"
                              style={{ fontSize: '0.8rem', padding: '0.2rem 0.5rem' }}
                              onClick={() => {
                                const parts = currentPath.split('/');
                                parts.pop();
                                const parent = parts.join('/');
                                setCurrentPath(parent);
                                fetchTree(selectedRepo.owner, selectedRepo.name, currentBranch, parent);
                              }}
                            >
                              &uarr; Up to {currentPath.split('/').slice(0, -1).join('/') || 'root'}
                            </button>
                          </div>
                        )}
                        <table className="tree-table">
                          <tbody>
                            {treeEntries.length === 0 ? (
                              <tr>
                                <td className="tree-cell" style={{ color: '#8b949e' }}>
                                  Empty repository or directory.
                                </td>
                              </tr>
                            ) : (
                              treeEntries.map((entry) => (
                                <tr
                                  key={entry.name}
                                  className="tree-row"
                                  onClick={() => handleFileClick(entry)}
                                >
                                  <td className="tree-cell">
                                    <span className="file-icon">{entry.is_dir ? '📁' : '📄'}</span>
                                    <span style={{ fontWeight: entry.is_dir ? 600 : 400, color: entry.is_dir ? '#58a6ff' : '#e6edf3' }}>
                                      {entry.name}
                                    </span>
                                  </td>
                                  <td className="tree-cell" style={{ textAlign: 'right', color: '#8b949e', fontFamily: 'monospace', fontSize: '0.8rem' }}>
                                    {entry.object_id.substring(0, 8)}
                                  </td>
                                </tr>
                              ))
                            )}
                          </tbody>
                        </table>
                      </div>
                    )}
                  </div>
                )}

                {/* Tab: Commits View */}
                {activeTab === 'commits' && (
                  <div>
                    {commits.length === 0 ? (
                      <p style={{ color: '#8b949e' }}>No commits yet on branch {currentBranch}.</p>
                    ) : (
                      commits.map((commit) => (
                        <div key={commit.id} className="commit-card">
                          <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                            <span className="commit-hash">{commit.id.substring(0, 12)}</span>
                            <span className="commit-meta">
                              {new Date(commit.timestamp_nanos / 1_000_000).toLocaleString()}
                            </span>
                          </div>
                          <div className="commit-msg">{commit.message}</div>
                          <div className="commit-meta">
                            {commit.author_name} &lt;{commit.author_email}&gt;
                          </div>
                        </div>
                      ))
                    )}
                  </div>
                )}
              </div>
            ) : (
              <p style={{ color: '#8b949e' }}>Select a repository to view contents.</p>
            )}
          </div>
        </div>
      )}

      {/* Auth Modal (Login / Register) */}
      {showAuthModal && (
        <div className="modal-backdrop" onClick={() => setShowAuthModal(false)}>
          <div className="modal-box" onClick={(e) => e.stopPropagation()}>
            <h2 style={{ marginTop: 0, marginBottom: '1.25rem', fontSize: '1.25rem' }}>
              {authMode === 'login' ? 'Sign In to Spine Forge' : 'Create Spine Forge Account'}
            </h2>

            {authError && (
              <div
                style={{
                  backgroundColor: '#3b1c1d',
                  border: '1px solid #f85149',
                  color: '#ff7b72',
                  padding: '0.6rem 0.8rem',
                  borderRadius: '6px',
                  marginBottom: '1rem',
                  fontSize: '0.85rem',
                }}
              >
                {authError}
              </div>
            )}

            <form onSubmit={handleAuthSubmit}>
              <div className="form-group">
                <label className="form-label">Username</label>
                <input
                  type="text"
                  className="form-input"
                  required
                  value={authForm.username}
                  onChange={(e) => setAuthForm({ ...authForm, username: e.target.value })}
                />
              </div>

              {authMode === 'register' && (
                <div className="form-group">
                  <label className="form-label">Email</label>
                  <input
                    type="email"
                    className="form-input"
                    required
                    value={authForm.email}
                    onChange={(e) => setAuthForm({ ...authForm, email: e.target.value })}
                  />
                </div>
              )}

              <div className="form-group">
                <label className="form-label">Password</label>
                <input
                  type="password"
                  className="form-input"
                  required
                  value={authForm.password}
                  onChange={(e) => setAuthForm({ ...authForm, password: e.target.value })}
                />
              </div>

              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginTop: '1.5rem' }}>
                <span
                  style={{ fontSize: '0.85rem', color: '#58a6ff', cursor: 'pointer' }}
                  onClick={() => {
                    setAuthMode(authMode === 'login' ? 'register' : 'login');
                    setAuthError(null);
                  }}
                >
                  {authMode === 'login' ? "Don't have an account? Register" : 'Already registered? Sign in'}
                </span>
                <div style={{ display: 'flex', gap: '0.5rem' }}>
                  <button type="button" className="btn btn-secondary" onClick={() => setShowAuthModal(false)}>
                    Cancel
                  </button>
                  <button type="submit" className="btn btn-primary">
                    {authMode === 'login' ? 'Sign In' : 'Register'}
                  </button>
                </div>
              </div>
            </form>
          </div>
        </div>
      )}

      {/* New Repository Modal */}
      {showNewRepoModal && (
        <div className="modal-backdrop" onClick={() => setShowNewRepoModal(false)}>
          <div className="modal-box" onClick={(e) => e.stopPropagation()}>
            <h2 style={{ marginTop: 0, marginBottom: '1.25rem', fontSize: '1.25rem' }}>
              Create a New Repository
            </h2>

            {createRepoError && (
              <div
                style={{
                  backgroundColor: '#3b1c1d',
                  border: '1px solid #f85149',
                  color: '#ff7b72',
                  padding: '0.6rem 0.8rem',
                  borderRadius: '6px',
                  marginBottom: '1rem',
                  fontSize: '0.85rem',
                }}
              >
                {createRepoError}
              </div>
            )}

            <form onSubmit={handleCreateRepo}>
              <div className="form-group">
                <label className="form-label">Repository Name</label>
                <input
                  type="text"
                  className="form-input"
                  required
                  placeholder="e.g. distributed-kv"
                  value={newRepoForm.name}
                  onChange={(e) => setNewRepoForm({ ...newRepoForm, name: e.target.value })}
                />
              </div>

              <div className="form-group">
                <label className="form-label">Description (optional)</label>
                <input
                  type="text"
                  className="form-input"
                  placeholder="Short summary of this repository"
                  value={newRepoForm.description}
                  onChange={(e) => setNewRepoForm({ ...newRepoForm, description: e.target.value })}
                />
              </div>

              <div className="form-group">
                <label className="form-label">Durability Policy</label>
                <select
                  className="form-input"
                  value={newRepoForm.durability}
                  onChange={(e) => setNewRepoForm({ ...newRepoForm, durability: e.target.value })}
                >
                  <option value="ThreeCopies">ThreeCopies (3 replicas, rack-redundant)</option>
                  <option value="TwoCopies">TwoCopies (2 replicas)</option>
                  <option value="SingleCopy">SingleCopy (1 replica, development only)</option>
                </select>
              </div>

              <div className="form-group">
                <label className="form-label">Visibility</label>
                <div className="radio-group">
                  <label style={{ fontSize: '0.9rem', cursor: 'pointer', display: 'flex', alignItems: 'center', gap: '0.4rem' }}>
                    <input
                      type="radio"
                      name="visibility"
                      checked={!newRepoForm.is_private}
                      onChange={() => setNewRepoForm({ ...newRepoForm, is_private: false })}
                    />
                    Public
                  </label>
                  <label style={{ fontSize: '0.9rem', cursor: 'pointer', display: 'flex', alignItems: 'center', gap: '0.4rem' }}>
                    <input
                      type="radio"
                      name="visibility"
                      checked={newRepoForm.is_private}
                      onChange={() => setNewRepoForm({ ...newRepoForm, is_private: true })}
                    />
                    Private
                  </label>
                </div>
              </div>

              <div style={{ display: 'flex', justifyContent: 'flex-end', gap: '0.5rem', marginTop: '1.5rem' }}>
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
    </div>
  );
};
