# ARCHITECTURE DECISION RECORD (ADR-0015)
## Native Git Interoperability, Archive Security, and Ecosystem Migration

- **Status:** Accepted  
- **Date:** 2026-11-04  
- **Deciders:** DeepMind Advanced Agentic Coding Pair Program  
- **Context:** Batch 15 — Native Git Interoperability, Repository Import/Export, Platform Migration, and Ecosystem Compatibility

---

## 1. CONTEXT & PROBLEM STATEMENT

Spine has established a complete, standalone distributed version-control system (`git-alternative/`) and code collaboration forge (`github-alternative/`). However, the software engineering industry is dominated by Git, GitHub, and GitLab. For Spine to achieve real-world adoption, organizations must be able to:

1. Import existing Git repositories into native Spine format with zero fidelity loss in tree and commit histories.
2. Export native Spine repositories to standard Git repositories so that teams retain full sovereignty over their code.
3. Migrate issues, pull requests, comments, labels, milestones, and releases from GitHub and GitLab export bundles.
4. Safely handle foreign archives without falling victim to Zip Slip, path traversal, Windows reserved device names, decompression bombs, or malicious automated workflows.

A common industry approach is to wrap the host `git` command-line tool or link against `libgit2` via C-FFI. We must decide how to architect Git interoperability in Spine.

---

## 2. DECISION DRIVERS

- **Memory Safety & Independence:** Spine is an original Rust platform; C-FFI bindings (`libgit2`) compromise memory safety and introduce platform-specific dynamic linking friction.
- **Production Isolation:** Calling external system binaries (`git.exe` / `/usr/bin/git`) creates environment-dependent failure modes, shell injection attack vectors, and process spawning overhead.
- **Auditable Format Verification:** Git format parsing must be transparent, testable, and strictly checked against Git specifications.
- **Security Posture:** External migration bundles must be treated as untrusted input; rogue file paths and scripts must be neutralized before hitting disk.

---

## 3. CONSIDERED ALTERNATIVES

- **Alternative A: Shell wrapper around host `git` CLI.**  
  *Pros:* Easy to write quickly.  
  *Cons:* Requires host Git installation; brittle across OS versions; prone to process failure and injection; violates Spine's self-contained architectural mandate.
- **Alternative B: C-FFI bindings to `libgit2`.**  
  *Pros:* Full Git compatibility layer.  
  *Cons:* C dependency in build pipeline; potential memory vulnerabilities (use-after-free, buffer overflows); complex cross-compilation on Windows MSVC.
- **Alternative C: Native Rust Git format parser and serializer (`spine-migration`).**  
  *Pros:* Zero external runtime dependencies; 100% memory-safe; full control over error handling and streaming; external `git` CLI can be retained strictly as a test oracle.  
  *Cons:* Requires implementing Git loose objects, canonical tree sorting, packfile v2, delta decoding, and pack indexing from scratch.

---

## 4. DECISION

We select **Alternative C: Native Rust Git format engine** implemented in `git-alternative/crates/spine-migration/`.

### Key Architectural Choices:

1. **Pure Rust Git Format Codec:**  
   Implement zlib-compressed loose objects, Git SHA-1 hashing, canonical tree sorting (with synthetic trailing `/` for directories), commit serialization, annotated tags, packfile v2 parsing, delta instruction execution (COPY and INSERT), and v2 pack indexing.
2. **Oracle Verification in Tests:**  
   Host `git` CLI is used strictly as an oracle inside `spine-migration/tests/git_interop_test.rs` to run `git fsck --full`, `git log`, and `git cat-file` against Spine-generated Git repositories. No production code calls `git`.
3. **Hardened Archive Extraction:**  
   `archive_security.rs` validates all incoming `.zip` and `.tar` entries. Any path resolving outside target directories, containing parent segments (`..`), or matching Windows reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1-9`, `LPT1-9`) is rejected.
4. **Fail-Closed Workflow Deactivation:**  
   All migrated CI/CD workflows from `.github/workflows` or `.gitlab-ci.yml` are marked inactive (`is_active: false`) upon import. Users must inspect and explicitly activate them.
5. **Safe User Attribution:**  
   External contributors are mapped to ghost tombstone accounts (`ghost-<username>`) with safe email aliases and zero repository administrative roles.

---

## 5. CONSEQUENCES

### Positive
- Fully self-contained Spine binary with zero runtime dependencies.
- Perfect deterministic fidelity verified by official Git's own `fsck` validator.
- Robust security against path traversal and decompression bomb exploits.
- Complete bidirectional migration: `Git -> Spine` and `Spine -> Git`.

### Negative / Trade-offs
- Packfile delta compression during export uses baseline object streams; heavy delta optimization can be expanded in future releases.
- Large repositories (> 100k commits) require streaming memory management during commit topological resolution.
