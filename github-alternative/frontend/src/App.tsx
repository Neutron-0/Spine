import React, { useEffect, useState } from 'react';
import { RepositoryMeta, CommitDto } from './types';

export const App: React.FC = () => {
  const [repos, setRepos] = useState<RepositoryMeta[]>([]);
  const [selectedRepo, setSelectedRepo] = useState<RepositoryMeta | null>(null);
  const [commits, setCommits] = useState<CommitDto[]>([]);
  const [loading, setLoading] = useState<boolean>(true);

  useEffect(() => {
    fetch('/api/v1/repos')
      .then((res) => {
        if (!res.ok) throw new Error(`HTTP error! status: ${res.status}`);
        return res.json();
      })
      .then((data: RepositoryMeta[]) => {
        setRepos(data);
        if (data.length > 0) {
          setSelectedRepo(data[0]);
        }
        setLoading(false);
      })
      .catch(() => {
        // Fallback mock data when running standalone without live backend
        const fallbackRepos: RepositoryMeta[] = [
          {
            id: 'repo_01j9',
            owner: 'spine-org',
            name: 'spine-core',
            description: 'High-performance Rust-native distributed version-control engine',
            is_private: false,
            default_branch: 'main',
            created_at: '2026-10-31T00:00:00Z',
          },
          {
            id: 'repo_02k1',
            owner: 'spine-org',
            name: 'spine-forge',
            description: 'Modern developer collaboration platform and distributed storage nodes',
            is_private: false,
            default_branch: 'main',
            created_at: '2026-10-31T00:00:00Z',
          },
        ];
        setRepos(fallbackRepos);
        setSelectedRepo(fallbackRepos[0]);
        setLoading(false);
      });
  }, []);

  useEffect(() => {
    if (!selectedRepo) return;
    fetch(`/api/v1/repos/${selectedRepo.owner}/${selectedRepo.name}/commits`)
      .then((res) => {
        if (!res.ok) throw new Error(`HTTP error: ${res.status}`);
        return res.json();
      })
      .then((data: CommitDto[]) => {
        setCommits(data);
      })
      .catch(() => {
        setCommits([
          {
            id: 'e4d909c290d0fb1ca068ffaddf22cbd0add22cbd0add22cbd0add22cbd0add22',
            author_name: 'Alice Engineer',
            author_email: 'alice@spine.org',
            timestamp: 1730332800000000000,
            message: 'feat: implement initial content-addressed storage engine',
          },
          {
            id: '1c2b3a4f5e6d7c8b9a0f1e2d3c4b5a6f7e8d9c0b1a2f3e4d5c6b7a8f9e0d1c2b',
            author_name: 'Bob Architect',
            author_email: 'bob@spine.org',
            timestamp: 1730332805000000000,
            message: 'docs: add repository binary format specification',
          },
        ]);
      });
  }, [selectedRepo]);

  return (
    <div className="container">
      <header className="header">
        <div className="logo">
          <span>SPINE FORGE</span>
          <span className="badge">v0.1.0-alpha</span>
        </div>
        <div>
          <span style={{ color: '#8b949e', fontSize: '0.9rem' }}>
            Powered by <strong>Spine VCS</strong> Engine (Rust + Axum + Tokio)
          </span>
        </div>
      </header>

      {loading ? (
        <p>Loading repositories...</p>
      ) : (
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 2fr', gap: '2rem' }}>
          <div>
            <h2 style={{ fontSize: '1.2rem', marginBottom: '1rem' }}>Repositories</h2>
            {repos.map((repo) => (
              <div
                key={repo.id}
                className="repo-card"
                style={{
                  borderColor: selectedRepo?.id === repo.id ? '#58a6ff' : undefined,
                }}
                onClick={() => setSelectedRepo(repo)}
              >
                <div className="repo-title">
                  {repo.owner}/{repo.name}
                </div>
                <div className="repo-desc">{repo.description}</div>
              </div>
            ))}
          </div>

          <div>
            {selectedRepo && (
              <div>
                <h2 style={{ fontSize: '1.2rem', marginBottom: '0.5rem' }}>
                  {selectedRepo.owner}/{selectedRepo.name}
                </h2>
                <p style={{ color: '#8b949e', marginBottom: '1.5rem' }}>
                  Branch: <code>{selectedRepo.default_branch}</code> | Visibility:{' '}
                  {selectedRepo.is_private ? 'Private' : 'Public'}
                </p>

                <h3 style={{ fontSize: '1.1rem', marginBottom: '1rem' }}>Recent Commits</h3>
                {commits.map((commit) => (
                  <div key={commit.id} className="commit-card">
                    <div className="commit-hash">{commit.id.substring(0, 10)}</div>
                    <div className="commit-msg">{commit.message}</div>
                    <div className="commit-meta">
                      {commit.author_name} &lt;{commit.author_email}&gt;
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
};
