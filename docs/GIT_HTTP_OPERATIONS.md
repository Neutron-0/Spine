# Git HTTP Operations Guide

## Using Official Git Clients with Spine Forge

Spine Forge seamlessly supports standard Git command line interfaces (version 2.x and newer) without third-party extensions or special drivers.

### 1. Cloning a Repository

#### Public Repository
Anonymous cloning requires no credentials:
```bash
git clone http://forge.example.com/alice/demo-project.git
```
*(The `.git` suffix is optional: `http://forge.example.com/alice/demo-project` also works.)*

#### Private Repository
Clone with an embedded Personal Access Token:
```bash
git clone http://alice:sp_pat_your_token_here@forge.example.com/alice/private-project.git
```
Or clone using credential helpers:
```bash
git clone http://forge.example.com/alice/private-project.git
# Username for 'http://forge.example.com': alice
# Password for 'http://alice@forge.example.com': sp_pat_your_token_here
```

### 2. Pushing Changes

Pushing requires write permissions on the target repository:
```bash
git remote add origin http://alice:sp_pat_your_token_here@forge.example.com/alice/demo-project.git
git push -u origin main
```

#### Pushing New Branches & Tags
```bash
git checkout -b feature/search-opt
git push origin feature/search-opt

git tag -a v1.2.0 -m "Release 1.2.0"
git push origin --tags
```

#### Deleting a Remote Branch
```bash
git push origin --delete feature/search-opt
```

### 3. Fetching and Pulling

```bash
git fetch origin
git pull origin main
```

### 4. Working with Branch Protection

If a branch is protected by the repository administrator (requiring a pull request):
```bash
git push origin main
# Remote rejected: push rejected: direct push to protected branch 'main' is prohibited; pull request required
```
To proceed:
1. Push your work to a feature branch:
   ```bash
   git push origin feature/my-change
   ```
2. Open a Pull Request in Spine Forge's Web UI.
3. Once approved, merge through Forge's authoritative write pipeline.

### 5. Secret Scanner Rejections

If a commit introduces API keys, Personal Access Tokens, or private keys:
```bash
git push origin main
# Remote rejected: push rejected: secret scanning detected sensitive credentials (Spine Personal Access Token)
```
To resolve:
1. Remove the sensitive secret from your commit history:
   ```bash
   git reset HEAD~1
   # Edit files to remove secrets
   git commit -am "Sanitized commit without credentials"
   git push origin main
   ```
