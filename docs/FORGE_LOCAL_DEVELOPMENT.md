# Spine Forge: Local Development Guide

## 1. Quick Start (Hermetic In-Memory Mode)

The fastest way to run Spine Forge locally without installing PostgreSQL or Docker is via the built-in in-memory database mode:

### Windows (PowerShell)
```powershell
.\scripts\dev_forge.ps1 -Action start -InMemory
```

### Linux / macOS (Bash)
```bash
./scripts/dev_forge.sh start
```

This starts:
- `spine-api` listening on `http://127.0.0.1:3000`.
- In-memory relational database (fully initialized with tables and indexes).
- Background `ProvisioningWorker` processing repository creation jobs.
- Local storage root under `.spine-dev-forge/storage`.

---

## 2. Docker Compose Environment (Full Multi-Node Cluster)

To run the complete production topology including PostgreSQL 16, Spine Coordinator, and 3 Storage Nodes:

```bash
docker-compose up -d
```

Services:
- **PostgreSQL**: `127.0.0.1:5432` (database `spine_forge`)
- **Coordinator**: `127.0.0.1:8400`
- **Storage Node 1**: `127.0.0.1:9101`
- **Storage Node 2**: `127.0.0.1:9102`
- **Storage Node 3**: `127.0.0.1:9103`
- **Spine API**: `http://127.0.0.1:3000`
- **Spine Frontend**: `http://127.0.0.1:5173`

---

## 3. Running the Frontend Locally

```bash
cd github-alternative/frontend
npm install
npm run dev
```

Open `http://127.0.0.1:5173` in your browser.

---

## 4. Testing with the Spine CLI

Once the API backend is running:

1. **Log In**:
   ```bash
   spine auth login --host http://127.0.0.1:3000
   ```
2. **Check Session**:
   ```bash
   spine auth status
   ```
3. **Create a Repository**:
   ```bash
   spine repo create demo-project --description "My first Spine Forge repo"
   ```
4. **Log Out**:
   ```bash
   spine auth logout --host http://127.0.0.1:3000
   ```
