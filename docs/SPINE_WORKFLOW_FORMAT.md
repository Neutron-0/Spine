# Spine Workflow Specification Format (.spine/workflows/*.yml)

## 1. Specification Overview

Spine workflows are declarative automation files stored in `.spine/workflows/` at the root of a repository. They define the triggers, environment variables, jobs, dependency graph, runner label requirements, and execution steps.

Files are parsed using Spine's native Rust YAML parser (`spine-workflow::parser::WorkflowParser`) and validated for syntax, type safety, and cycle-free DAG topology.

---

## 2. Top-Level Schema

```yaml
name: "Continuous Integration & Test"
on:
  push:
    branches: [main, "release/*"]
  pull_request:
    branches: [main]
  workflow_dispatch: true

env:
  CARGO_TERM_COLOR: "always"
  RUST_BACKTRACE: "1"

jobs:
  lint:
    name: "Clippy & Formatting"
    runs-on: [self-hosted, windows]
    timeout-minutes: 10
    steps:
      - name: "Checkout Code"
        run: "echo Checking out source tree..."
      - name: "Run Clippy"
        run: "cargo clippy --all-targets -- -D warnings"

  test:
    name: "Unit & Integration Tests"
    needs: [lint]
    runs-on: [self-hosted, windows]
    steps:
      - name: "Run Test Suite"
        run: |
          echo "Starting test suite..."
          cargo test --workspace

  build:
    name: "Release Compilation"
    needs: [test]
    runs-on: [self-hosted, windows]
    steps:
      - name: "Build Release Binaries"
        run: "cargo build --release"
```

---

## 3. Field Definitions

### 3.1 `name` (string, required)
A human-readable identifier for the workflow. Displayed in the Forge UI, commit status checks, and audit trails.

### 3.2 `on` (TriggerSpec, required)
Defines events that trigger workflow execution:
- `push`:
  - `branches`: List of branch names or glob patterns (`main`, `release/*`, `*`).
  - `paths`: Optional list of file path patterns to limit execution.
- `pull_request`:
  - `branches`: List of target base branches.
- `workflow_dispatch`: Boolean enabling manual triggering from the Forge Web UI or REST API.

### 3.3 `env` (map<string, string>, optional)
Workflow-wide environment variables made available to all jobs and steps.

### 3.4 `jobs` (map<string, JobSpec>, required)
A dictionary mapping unique job keys to job specifications:
- `name` (string, optional): Display name for the job. Defaults to the job key.
- `runs-on` (list<string>, required): Labels identifying compatible runners (e.g., `[windows, self-hosted]`).
- `needs` (list<string>, optional): Direct dependencies required before this job can execute.
- `timeout-minutes` (integer, optional): Maximum execution time allowed before the runner is terminated (default: 60).
- `steps` (list<StepSpec>, required): Sequential list of execution commands.

### 3.5 `steps` (list of StepSpec)
- `name` (string, optional): Step title displayed in logs.
- `run` (string): Inline or multiline shell command executed in the runner's isolated workspace.
- `uses` (string, optional): Pre-packaged action specification (e.g., `actions/checkout@v1`).
- `env` (map<string, string>, optional): Step-specific environment variable overrides.

---

## 4. Native Pre-Injected Variables

The runner automatically injects the following standard environment variables into every step:
- `SPINE_RUN_ID`: Unique UUID of the workflow run.
- `SPINE_JOB_ID`: Unique UUID of the current job.
- `SPINE_JOB_KEY`: Declared key of the job within the workflow.
- `SPINE_WORKSPACE`: Canonical file path to the isolated sandbox workspace.
- Configured repository and organization secrets (e.g. `SPINE_API_SECRET`).
