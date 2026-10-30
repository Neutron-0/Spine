# SPINE DEVELOPER GUIDE

## 1. Prerequisites
- Rust stable toolchain 1.80+ (`rustc`, `cargo`)
- Node.js v20+ and npm
- Windows PowerShell, Linux bash, or macOS zsh

## 2. Directory Layout
- `git-alternative/`: Spine VCS engine and CLI.
- `github-alternative/`: Spine Forge collaboration platform.
  - `backend/`: Axum API server, storage nodes, background workers.
  - `frontend/`: React + TypeScript + Vite web application.
- `references/`: Isolated third-party research reference repositories.

## 3. Building the Version Control System (Spine VCS)
```powershell
cd git-alternative
cargo build --release
```
To run tests:
```powershell
cargo test
```
To run benchmarks:
```powershell
cargo bench
```

## 4. Running Spine Forge Platform

### Backend
```powershell
cd github-alternative/backend
cargo run
```

### Frontend
```powershell
cd github-alternative/frontend
npm install
npm run dev
```
