# ADR-0005: Spine Wire Protocol — Binary Framing, Graph Negotiation, Streaming Pack Transfer, and Atomic CAS Push

## Status
Accepted

## Date
2026-10-31

## Context
Following the completion of Storage Engine V2 in Batch 03 (featuring `.spk` packfiles, 256-bucket fanout `.spidx` indexes, and generational catalog compaction), Spine is a fully featured, self-contained local version-control engine. To evolve into an independent distributed version-control system capable of powering collaborative workflows, Spine requires a native network protocol.

### Deficiencies of Conventional Protocols (Git Smart HTTP / Packet-Line)
Git's networking model was designed incrementally between 2005 and 2018 and exhibits several structural flaws:
1. **Ad-Hoc Text/Binary Hybrids:** The Git `pkt-line` format prefixes payloads with 4 hexadecimal ASCII characters indicating byte length (e.g. `000afoo\n`). It suffers from redundant ASCII parsing, strict 65,520-byte payload limits, and null-byte delimiter hacks.
2. **Side-Band Inband Multiplexing:** Side-band channel multiplexing (`side-band-64k`) injects 1-byte channel indicators (1 = pack data, 2 = progress, 3 = error) directly inside payload packets, entangling application framing with network transport.
3. **Unauthenticated Reference Leakage:** Git protocols advertise all remote references, branch names, and object IDs *before* client authentication is verified, exposing internal repository structure to unauthenticated network observers.
4. **Vulnerable Delta Negotiation:** Git's negotiation protocol involves exchanging commit IDs in conversational chat loops over high-latency links without explicit session tokens, creating vulnerability to replay attacks and high round-trip overhead.
5. **Lack of Native Protocol Security:** Git Smart HTTP offloads authentication entirely to Web server HTTP basic/digest headers or OAuth reverse proxies, without cryptographic challenge-response binding to the active session.

### Requirements for Spine Wire Protocol (`SPWP`)
Spine requires an original, native wire protocol that:
- Uses a fixed, 64-bit aligned 16-byte binary header with zero text parsing overhead.
- Integrates cryptographic authentication directly into the handshake before any repository metadata or branch references are advertised.
- Transports compressed object records directly using Spine's `.spk` packfile stream, avoiding recompression or repack overhead between wire and disk.
- Enforces atomic compare-and-swap (CAS) push operations with strict ancestor reachability checks to eliminate race conditions during concurrent pushes.
- Resists network disconnection, packet corruption, and partial stream interruptions through transactional staging and BLAKE3 verification.

---

## Decision

### 1. Protocol Architecture & URL Scheme
Spine defines its native transport protocol identified by the URL scheme:
`spine://[user@]hostname[:port]/repository-path`

- Default TCP port: **4433** (TLS 1.3 encrypted), fallback port **4434** for cleartext loopback testing/intranet daemon.
- Layering: Standard TCP transport $\to$ TLS 1.3 / Plaintext Framing $\to$ `SPWP` Frame Demuxer $\to$ Protocol State Machine.

### 2. Binary Framing Specification (`SPWP`)
Every frame transmitted across a Spine network connection begins with a strict 16-byte header:

```
+-----------------------------------------------------------------------+
|  0..3   | Magic Bytes: b"SPWP" (0x53, 0x50, 0x57, 0x50)               |
+-----------------------------------------------------------------------+
|  4..5   | Protocol Version: u16 LE (0x0001)                           |
+-----------------------------------------------------------------------+
|  6..7   | Frame Type: u16 LE (Registry below)                         |
+-----------------------------------------------------------------------+
|  8..9   | Flags: u16 LE (0x0001 = Compressed, 0x0002 = EndOfStream)   |
+-----------------------------------------------------------------------+
| 10..13  | Payload Length: u32 LE (Bounded <= 16,777,216 bytes)         |
+-----------------------------------------------------------------------+
| 14..17  | Payload BLAKE3 Checksum: 4 bytes (Truncated from 32-byte)    |
+-----------------------------------------------------------------------+
| 18..18+L| Variable-length Payload: L bytes                            |
+-----------------------------------------------------------------------+
```

Total Header Size: 18 bytes (including 4-byte payload checksum).

### 3. Frame Type Registry
```
0x0001: HELLO                (Client initial capabilities advertisement)
0x0002: HELLO_ACK            (Server accepted version and capabilities)
0x0010: AUTH_CHALLENGE       (Server random 32-byte nonce challenge)
0x0011: AUTH_RESPONSE        (Client Ed25519 signature + public key)
0x0012: AUTH_RESULT          (Server authorization outcome & permissions)
0x0020: REPO_OPEN            (Client selects target repository path)
0x0021: REPO_INFO            (Server returns repo algorithm, default branch)
0x0030: REF_ADVERTISE        (Server advertises tips: branches and tags)
0x0031: REF_UPDATE_REQ       (Client proposes CAS reference updates)
0x0032: REF_UPDATE_RES       (Server returns per-ref update status)
0x0040: FETCH_NEGOTIATE      (Client sends WANT and HAVE lists)
0x0041: FETCH_PLAN           (Server returns pack summary: object count, size)
0x0050: PUSH_BEGIN           (Client initiates push pack transmission)
0x0051: PACK_DATA            (Chunk of binary .spk packfile stream)
0x0052: PACK_PROGRESS        (Out-of-band progress feedback to client)
0x0053: PACK_END             (Final pack trailer with global checksum)
0x0060: ERROR                (Structured error code + message)
0x0070: PING                 (Keepalive probe)
0x0071: PONG                 (Keepalive acknowledgement)
```

### 4. Connection State Machine
1. **Handshake & Capability Exchange:** Client sends `HELLO` containing protocol version (1), client implementation identifier, and supported capabilities. Server responds with `HELLO_ACK`.
2. **Mutual/Peer Authentication:** Server issues `AUTH_CHALLENGE` with a cryptographically secure 32-byte random nonce and timestamp. Client responds with `AUTH_RESPONSE` containing client public key, identity, and Ed25519 signature over `(nonce || repo_path)`. Server validates and responds with `AUTH_RESULT` granting Read, Write, or Admin scope.
3. **Repository Selection:** Client sends `REPO_OPEN(path)`. Server opens the target `.spine` repository, checks repository lock, and replies with `REPO_INFO` detailing object hash algorithm (`Blake3` or `Sha256`), default branch, and capabilities.
4. **Reference Advertisement:** Server transmits `REF_ADVERTISE` containing all branches (`refs/heads/*`) and tags (`refs/tags/*`) with their current target `ObjectId`s.
5. **Graph Negotiation (Fetch / Clone):**
   - In a clone or fetch, client sends `FETCH_NEGOTIATE` containing:
     - `WANT`: Object IDs of desired remote branch tips.
     - `HAVE`: Object IDs of commits the client already possesses locally.
   - The server traverses its commit history DAG backwards starting at `WANT` and terminating traversal at all paths reaching any `HAVE` commit.
   - The server collects all reachable commits, trees, and blobs that are missing on the client.
6. **Streaming Pack Transfer:**
   - The server streams the missing objects as a sequence of `PACK_DATA` frames.
   - The frames compose a valid Spine `.spk` packfile stream.
   - The client writes the incoming chunks directly into a temporary staging file `.spine/objects/pack/.incoming-<uuid>.spk`.
   - Once `PACK_END` arrives with the cumulative BLAKE3 checksum, the client validates the file, generates the corresponding `.spidx` 256-bucket index, renames both into active storage, registers the pack in `catalog.json`, and updates local remote-tracking references (`refs/remotes/<remote>/<branch>`).
7. **Atomic Compare-and-Swap Push:**
   - To update remote references, the client sends `PUSH_BEGIN`.
   - The client streams missing objects in `PACK_DATA` frames to the server.
   - The server receives the pack into `.incoming-<uuid>.spk`, validates checksum and object reachability, and builds `.spidx`.
   - The client issues `REF_UPDATE_REQ` containing a list of `(ref_name, old_oid, new_oid)`.
   - The server locks references and executes atomic compare-and-swap:
     - For each ref: if `current_ref != old_oid`, the update is rejected with `STALE_REFERENCE`.
     - If fast-forward check fails (and force flag is false), the update is rejected with `NON_FAST_FORWARD`.
     - If all validations pass, references are updated atomically, and `catalog.json` registers the new pack.
     - Server replies with `REF_UPDATE_RES(status = OK)`.

---

## Consequences

### Positive
- **High Performance:** Binary framing eliminates text parsing overhead. Direct packfile streaming allows server-to-client and client-to-server transfers without decompressing or re-encoding objects.
- **Strong Security:** Zero metadata leakage prior to cryptographic authentication. Ed25519 signatures over monotonic nonces eliminate replay attacks.
- **Race Condition Immunity:** Atomic compare-and-swap reference updates guarantee that concurrent pushes cannot overwrite or corrupt repository tips.
- **Clean Transport Decoupling:** `SPWP` operates over any byte stream (TCP, TLS, Unix Domain Sockets, SSH tunnels) with identical framing.

### Negative
- Not compatible with legacy Git HTTP / Smart HTTP daemons. Git clients cannot directly pull from or push to a Spine daemon without an external translation gateway.
