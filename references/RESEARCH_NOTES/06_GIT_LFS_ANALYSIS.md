# RESEARCH NOTE 06: GIT LFS ARCHITECTURE ANALYSIS

## 1. Core Model & Philosophy
Git LFS solves large-file repository bloat by replacing large binaries with small pointer text files in the Git tree:
- **Pointer File Specification:**
  ```text
  version https://git-lfs.github.com/spec/v1
  oid sha256:4d7a214614ab2935c943f9e0ff69d22eadbb8f32b121a2124e0b47b584d3d4ec
  size 12345678
  ```
- **Filter Clean/Smudge:** Uses Git's `clean` filter on commit to upload large objects to an external HTTP server and write the pointer; uses `smudge` filter on checkout to download payload from HTTP into working copy.
- **Batch API:** A JSON REST protocol negotiating pre-signed S3/HTTP upload/download URLs.

## 2. Key Subsystems Inspected
- `lfs/pointer.go`: Strict parsing and formatting of pointer metadata.
- `tq/transfer_queue.go`: Concurrent worker pool uploading and downloading binary chunks.

## 3. What Spine Adopts
1. **Separation of Metadata and Bulk Payloads:** Keeping the core Merkle DAG light and fast while bulk assets are addressed by hash.
2. **Chunked Streaming Transfer Protocol:** HTTP streaming with pre-signed chunk validation for the Forge storage nodes.

## 4. What Spine Avoids
1. **External Process Shims (Clean/Smudge Filters):** Git LFS requires shelling out to a Go executable on every file read/write, adding noticeable latency on Windows. Spine natively integrates large-file handling into the core engine (`spine-object-store`).
