# ADR-0002: Versioned Multi-Parent Commit Architecture & Backwards Compatibility

## Status
Accepted

## Date
2026-10-31

## Context
In Batch 01, Spine introduced its initial commit object representation. To support rich version-control workflows—specifically three-way merges and future octopus merges—commits must support zero, one, or multiple parent commits in a strictly ordered and deterministic manner:
- **Root Commit:** 0 parents.
- **Ordinary Commit:** 1 parent (previous branch tip).
- **Merge Commit:** 2 parents:
  - Parent 1 (`OURS`): The previous tip of the target branch being merged into.
  - Parent 2 (`THEIRS`): The tip of the source branch being merged.
- **Future Multi-Parent:** $N$ parents ($N \ge 3$).

Furthermore, commits must track explicit committer metadata distinct from author metadata (e.g. when applying patches, rebasing, or executing merges), and the binary encoding must be explicitly versioned while maintaining 100% backwards compatibility for existing Batch 01 repositories.

## Decision

1. **Commit Structure:**
   Extend `Commit` in `spine-core` to include:
   - `format_version: u16` (version 2 for all newly generated commits).
   - `root_tree: ObjectId` (32 bytes).
   - `parents: Vec<ObjectId>` (ordered vector of parent commit IDs).
   - `author_name: String` & `author_email: String`.
   - `committer_name: String` & `committer_email: String`.
   - `timestamp_nanos: i64` (UTC nanoseconds).
   - `message: String`.

2. **Binary Framing (Commit v2):**
   - **Magic:** `b"SPCM"` (4 bytes, `0x53 0x50 0x43 0x4D`).
   - **Version:** `0x0002` (u16 LE).
   - **Payload:** Followed by root tree, 16-bit parent count, parent IDs, author fields, committer fields, timestamp, and message.

3. **Backwards Compatibility (Commit v1 Decoding):**
   - If a commit payload does not begin with `b"SPCM"`, it is detected and parsed under the legacy v1 format.
   - For v1 commits, `format_version` is assigned `1`, and `committer_name` / `committer_email` default to the author's identity.
   - This ensures existing Batch 01 repositories remain readable without data migration.

4. **Parent Ordering Invariant:**
   - For any merge commit, `parents[0]` is invariant as the checkout branch (`OURS`).
   - `parents[1]` is invariant as the incoming branch (`THEIRS`).

5. **Hash Algorithm Identity:**
   - The repository's hash algorithm is locked in `.spine/spine.toml` at initialization (`blake3` default or `sha256`).
   - Changing `hash_algorithm` in an existing repository containing objects is strictly prohibited and returns an error.

## Consequences
- **Positive:**
  - Full support for clean three-way merges with correct first-parent traversal semantics (`spine log --first-parent`).
  - Separates author and committer identities.
  - Seamless, non-destructive backwards compatibility with existing repositories.
  - Unambiguous format identification with error handling for unsupported future versions.
- **Negative / Mitigations:**
  - Slight increase in commit header size (4-byte magic + 2-byte version + committer fields), mitigated by high Zstandard compression ratios in loose storage.
