# Spine Remote Synchronization Architecture

## 1. Overview
Spine remote synchronization coordinates repository state between local working copies and remote hosting daemons. Unlike centralized systems, synchronization in Spine is strictly asynchronous, graph-theoretic, and decentralized:
- **Clone:** Replicates the complete reachable object graph from remote default tips into a fresh local repository, binds `origin`, and checks out the default branch.
- **Fetch:** Discovers upstream reference updates, determines the minimal missing object set via `WANT`/`HAVE` DAG traversal, streams missing objects into local `.spk` packs, and updates remote tracking references (`refs/remotes/<remote>/<branch>`).
- **Push:** Determines local commits not present on remote, packages missing objects into an atomic `.spk` pack stream, uploads to remote, and executes an atomic compare-and-swap reference update.
- **Pull:** Composes `fetch` with an automatic fast-forward integration into the active local branch, or halts with clean diagnostics if branches have diverged.

---

## 2. Graph Traversal and Object Delta Calculation (`WANT` / `HAVE`)

When synchronizing, bandwidth efficiency requires transmitting *only* objects that the receiver does not already have.

```
Remote Graph (Server)                     Local Graph (Client)
      C4 (WANT)                                    --
      |                                            |
      C3                                           --
      |                                            |
      C2 (Common ancestor) <----------------- C2 (HAVE)
      |                                            |
      C1                                           C1
```

### 2.1 Negotiation Algorithm
1. **WANT Set:** Receiver lists the target commit `ObjectId`s it wishes to acquire (e.g. current remote branch tips).
2. **HAVE Set:** Receiver lists its known local commit tips (from `refs/heads/*` and `refs/remotes/*`).
3. **Graph Walk:**
   - Server initializes a priority queue of commits starting with `WANT`.
   - Traverses parent pointers in reverse topological order (latest commits first).
   - If a commit is contained within `HAVE`, traversal terminates along that branch.
   - For all newly discovered commits (between `WANT` and `HAVE`), the server enumerates their referenced root trees and constituent blobs.
   - The server filters out any trees or blobs that were already reachable from any commit in `HAVE`.
4. **Resulting Delta Set:** The minimal closed subgraph of commits, trees, and blobs required to make the `WANT` set fully readable and intact on the client.

---

## 3. Atomic Compare-and-Swap Push Semantics

Spine push operations are strictly transactional and race-free:

1. **Reference Discovery:** Client inspects server's current advertised reference $R$ with tip $O_{old}$.
2. **Object Transmission:** Client streams all missing commits, trees, and blobs needed to support new tip $O_{new}$ into the server's temporary pack staging directory (`.incoming-<uuid>.spk`).
3. **Server Ingestion Verification:**
   - Server validates packfile framing and cumulative BLAKE3 checksum.
   - Server verifies that all objects are intact and decodable.
   - Server constructs the `.spidx` 256-bucket index.
4. **Fast-Forward & Reachability Check:**
   - Server verifies that $O_{old}$ is an ancestor of $O_{new}$ in the commit graph (unless `force = true` is explicitly granted).
   - If $O_{old}$ is not reachable from $O_{new}$, server rejects the push with `NON_FAST_FORWARD`.
5. **Atomic Compare-and-Swap:**
   - Server acquires `.spine/LOCK`.
   - Server verifies $R$ still equals $O_{old}$. If $R$ changed concurrently to $O_{concurrent}$, server returns `STALE_REFERENCE` and discards the pack.
   - Server updates $R \to O_{new}$ atomically.
   - Server registers `.spk` and `.spidx` into `catalog.json`.
   - Server releases lock and replies with success.

---

## 4. Remote Configuration Storage
Remotes are configured in `.spine/spine.toml` under the `[remote.<name>]` section:

```toml
[remote.origin]
url = "spine://127.0.0.1:4434/team/project"
fetch = "+refs/heads/*:refs/remotes/origin/*"
push_default = "current"
```

Tracking references are stored cleanly under `.spine/refs/remotes/<remote>/<branch>`.
