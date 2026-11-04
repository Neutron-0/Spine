# ARTIFACT GARBAGE COLLECTION AND RETENTION

## 1. Tracing Model

Artifact garbage collection is performed via mark-and-sweep reachability tracing using `ArtifactGcCollector`:

```
                 Root Set (Reachable Artifacts)
   ┌───────────────────┬───────────────────┬───────────────────┐
   │                   │                   │                   │
   ▼                   ▼                   ▼                   ▼
Active Commits    Forge Releases     Generic Artifacts    Package Versions
(LFS Pointers)     (Asset OIDs)        (Namespace OIDs)     (.crate OIDs)
   │                   │                   │                   │
   └───────────────────┴───────────────────┴───────────────────┘
                               │
                               ▼
               Active ArtifactManifests Extracted
                               │
            Expand Referenced Chunk Hashes (Marked)
                               │
                               ▼
            Candidate Chunks = (All Chunks - Marked Chunks)
                               │
         Check Ingestion Grace Period (Default: 24 Hours)
                               │
                               ▼
            Sweep: Purge Unreferenced Expired Chunks
```

---

## 2. Ingestion Grace Period

To prevent race conditions where a chunk is uploaded in an active session but not yet linked to a committed manifest:
- Chunks have an creation timestamp metadata attribute.
- Chunks younger than the grace period (default 24 hours) are **never** deleted by GC, even if unreferenced by known manifests.

---

## 3. Pruning Semantics

- **Dry Run Mode**: `spine lfs prune --dry-run` calculates and outputs the number of reachable chunks, orphan candidates, and potential reclaimed megabytes without performing deletions.
- **Execution Mode**: `spine lfs prune` purges orphan chunk files from disk and returns an exact summary report.
