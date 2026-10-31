# Spine Network Failure and Recovery Specification

## 1. Failure Modes and Recovery Invariants

Distributed version control systems operate over inherently unpredictable network infrastructure: Wi-Fi handoffs, dropped connections, high packet loss, firewall resets, and server process crashes.

Spine enforces three non-negotiable invariants during all network failures:
1. **Never Corrupt the Host Repository:** A client or server crash during synchronization must NEVER leave the recipient repository in an unreadable or half-written state.
2. **Zero Incomplete Object References:** A branch reference pointer is NEVER updated until all prerequisite objects are physically synced to storage and registered in `catalog.json`.
3. **Automatic Staging Cleanup:** Incomplete streams or failed transfers are staged in transient files and purged without requiring manual user intervention.

---

## 2. Failure Handling by Phase

### 2.1 Connection Drop During Handshake / Authentication
- **Symptom:** TCP reset or socket EOF during `HELLO` or `AUTH_RESPONSE`.
- **System Action:** Server deallocates the connection state machine. No repository locks or files were touched; zero residual state.

### 2.2 Connection Drop During Pack Streaming
- **Symptom:** Client or server socket disconnects while transmitting `PACK_DATA` frames.
- **Recipient Action:**
  - Receiver was writing chunks into `.spine/objects/pack/.incoming-<uuid>.spk`.
  - When socket returns EOF prematurely before receiving `PACK_END`:
    - Receiver closes the temporary file handle.
    - Receiver unlinks `.incoming-<uuid>.spk`.
    - No changes are written to `catalog.json` or `refs/`.
  - The repository remains 100% clean and consistent.

### 2.3 Checksum Mismatch or Corrupted Stream
- **Symptom:** Receiver reads complete pack, but cumulative BLAKE3 checksum or record checksum does not match expected value in `PACK_END`.
- **Recipient Action:**
  - Receiver immediately discards and deletes `.incoming-<uuid>.spk`.
  - Returns `0x0060 ERROR (CorruptionDetected)` to sender.
  - No references or catalogs are modified.

### 2.4 Server Crash During Push Compare-and-Swap
- **Symptom:** Server process dies while verifying or updating references.
- **Recovery:**
  - The `.spine/LOCK` file contains the PID of the dead server process.
  - When the daemon restarts or the next client connects, `RepoLock::acquire` checks the dead PID, breaks the stale lock cleanly, and resumes normal operations.
  - Unregistered `.incoming-*` packs in `.spine/objects/pack/` older than 1 hour are cleaned up during background sweeps.

### 2.5 Resuming Interrupted Synchronizations
Because Spine uses immutable content-addressed packfiles and object IDs:
- In the next synchronization attempt, the client advertises all objects it already holds in its `HAVE` list.
- If part of the object graph was already committed in a previous pack, the server excludes those objects from the new plan.
- Only the truly remaining objects are packaged and sent.
