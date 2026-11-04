# Git Packfile Transfer and Ingestion

## Packfile Format & Processing

Spine Forge natively processes Git Packfile version 2 format (`.pack`) and its accompanying 20-byte SHA-1 trailer.

### 1. Ingestion (`git-receive-pack`)

1. **Pack Header Verification:**
   - 4-byte signature: `PACK` (0x50, 0x41, 0x43, 0x4b).
   - 4-byte version: 2 (0x00, 0x00, 0x00, 0x02).
   - 4-byte object count: $N$.

2. **Thin Pack Delta Resolution:**
   Git push clients routinely emit thin packfiles containing `OBJ_REF_DELTA` or `OBJ_OFS_DELTA` referring to base objects assumed to exist on the remote.
   - Spine Forge's `parse_packfile_with_resolver` queries the repository's persisted Git object cache and native object store.
   - External base objects are dynamically resolved, decompressed, and applied using Git delta instruction interpreters.

3. **Spine Object Store Translation:**
   - Git blobs $\rightarrow$ Native Spine loose/pack blobs.
   - Git trees $\rightarrow$ Native Spine trees (`TreeEntry` with canonical Spine `FileMode`).
   - Git commits $\rightarrow$ Native Spine commits (`Commit` with author, committer, nanosecond timestamps, and parent array).
   - Git annotated tags $\rightarrow$ Native Spine annotated tags (`AnnotatedTag`).

4. **Raw Object Caching:**
   - To guarantee bit-for-bit SHA-1 round-trip identity across disparate Git clients, raw Git commit and tree objects are cached in `.spine/git_objects/`.
   - The bijection $f: \text{GitOid} \leftrightarrow \text{ObjectId}$ is persisted in `.spine/git_map.json`.

### 2. Pack Generation (`git-upload-pack`)

1. **Reachable Object Collection:**
   Given a set of requested commit OIDs (`want <oid>`):
   - Traverse the directed acyclic graph (DAG) starting at each want.
   - If raw Git objects are cached, reuse the exact original payload.
   - If objects originate natively from Spine, synthesize canonical Git trees and commits using `GitExporter`.

2. **Pack Index & Packfile Construction:**
   - Objects are assembled into uncompressed and zlib-deflated streams.
   - SHA-1 checksum trailer is computed and appended.
   - Packfile is partitioned and framed in sideband channel 1 packets (`side-band-64k`).
