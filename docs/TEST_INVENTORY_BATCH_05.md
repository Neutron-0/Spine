# Spine VCS Test Inventory & Reconciliation Report (Batch 05 Baseline)

## 1. Reconciliation of Batch 04 Test Counts

In the Batch 04 engineering report, an apparent discrepancy was noted between the summary headline ("49 passing tests") and the detailed test inventory table, which enumerated:
- 40 library crate unit tests
- 15 binary integration tests
- **Total: 55 passing tests**

### Root Cause Analysis
During Batch 04 execution, prior to the addition of `tests/network_remote_test.rs` (3 integration tests) and `test_stale_lock_recovery` / new protocol tests, an earlier milestone count of 49 was recorded. When the full network test suite was committed, the summary text retained the intermediate figure while the detailed breakdown accurately listed all 55 tests.

### Verification Command & Environment
Execution command:
```powershell
cargo test --workspace --manifest-path git-alternative/Cargo.toml
```
All **55 tests** execute and pass with 0 failures, 0 errors, and 0 ignored tests.

---

## 2. Complete Inventory of Discovered and Executed Tests

### A. Integration Tests (`apps/spine-cli/tests/`) — 15 Tests
| Binary | Test Function | Status | Description |
|---|---|---|---|
| `branch_switch_test.rs` | `test_branch_management_and_safe_switching` | PASS | Branch creation, deletion, renaming, dirty worktree safety check, atomic switch |
| `cli_e2e_test.rs` | `test_cli_end_to_end_binary_workflow` | PASS | End-to-end CLI workflow: init, config, add, status, commit, log, show |
| `journal_recovery_test.rs` | `test_journal_crash_recovery_on_fsck` | PASS | Automatic journal recovery upon fsck invocation |
| `journal_recovery_test.rs` | `test_undo_and_redo_workflow` | PASS | Linear history undo and redo transactional state machine |
| `lifecycle_test.rs` | `test_end_to_end_repository_lifecycle` | PASS | Repository initialization, staged commit, branch, and status tracking |
| `merge_test.rs` | `test_diff_commands` | PASS | Unified line diff, staged diff, Myers LCS diff engine |
| `merge_test.rs` | `test_fast_forward_merge` | PASS | Fast-forward merge updating HEAD and working tree |
| `merge_test.rs` | `test_clean_3way_merge` | PASS | Clean non-overlapping 3-way tree and file merge |
| `merge_test.rs` | `test_conflict_resolution_with_legitimate_conflict_marker_content` | PASS | Text conflict marking, manual conflict marker resolution |
| `merge_test.rs` | `test_conflicted_merge_abort_and_continue` | PASS | Merge abort and merge continuation state transitions |
| `network_remote_test.rs` | `test_remote_configuration_and_tags` | PASS | `spine remote` (add, list, get-url, set-url, remove) and `spine tag` (create, list, delete) |
| `network_remote_test.rs` | `test_loopback_clone_push_fetch_pull_and_cas_conflict` | PASS | Full clone, push, collaborator clone/push, fast-forward pull, CAS conflict rejection, force push |
| `network_remote_test.rs` | `test_interrupted_pack_transfer_cleanup` | PASS | Truncated pack stream cleanup; zero partial pack leakage |
| `storage_v2_pack_gc_test.rs` | `test_garbage_collection_lifecycle_and_grace_period` | PASS | Reachability walk, orphan detection, 14-day grace period enforcement |
| `storage_v2_pack_gc_test.rs` | `test_storage_v2_repack_pack_inspect_and_composite_traversal` | PASS | Loose-to-pack repacking, .spidx 256-bucket fanout, composite store transparency |

### B. Unit Tests in Crates (`crates/`) — 40 Tests
| Crate | Test Function | Status | Description |
|---|---|---|---|
| `spine-config` | `test_config_set_get` | PASS | TOML config parsing, key-value mutations |
| `spine-config` | `test_immutable_hash_algorithm_protection` | PASS | Immutability check for repository hash algorithm |
| `spine-core` | `test_domain_separation_prevents_collision` | PASS | BLAKE3 domain separation across object types |
| `spine-core` | `test_hash_algorithm_blake3_vs_sha256` | PASS | BLAKE3 and SHA-256 algorithm agility |
| `spine-core` | `test_object_id_hex_roundtrip` | PASS | 32-byte ObjectId string and binary conversions |
| `spine-diff` | `test_diff_operations` | PASS | Line diff additions, deletions, context |
| `spine-diff` | `test_compare_trees_and_stats` | PASS | Tree comparisons and diffstat metrics |
| `spine-format` | `test_commit_corrupted_payload_rejection` | PASS | Corrupted commit payload detection |
| `spine-format` | `test_commit_serialization_roundtrip_v2` | PASS | V2 commit format serialization/deserialization |
| `spine-format` | `test_commit_unsupported_version_rejection` | PASS | Unsupported commit version rejection |
| `spine-format` | `test_commit_v1_legacy_compatibility` | PASS | V1 legacy commit backward compatibility |
| `spine-format` | `test_loose_header_roundtrip` | PASS | Loose object header serialization |
| `spine-format` | `test_tree_serialization_roundtrip` | PASS | Tree directory entry binary encoding |
| `spine-history` | `test_commit_graph_walk` | PASS | Topological commit graph traversal |
| `spine-history` | `test_criss_cross_merge_base` | PASS | Criss-cross history merge base resolution |
| `spine-history` | `test_divergent_branch_merge_base` | PASS | Lowest common ancestor on divergent histories |
| `spine-index` | `test_corrupt_index_checksum_fails` | PASS | Binary staging index CRC32/BLAKE3 checksum validation |
| `spine-index` | `test_index_serialization_and_checksum_roundtrip` | PASS | Staging index binary format roundtrip |
| `spine-integrity` | *(None defined in lib.rs)* | N/A | Integrity checked via `fsck` integration tests |
| `spine-journal` | `test_journal_crash_recovery` | PASS | Write-ahead journal recovery after simulated crash |
| `spine-journal` | `test_journal_prepare_and_commit` | PASS | Two-phase transaction prepare and commit |
| `spine-journal` | `test_journal_state_machine_applying_and_recovery_required` | PASS | Journal recovery state machine transitions |
| `spine-journal` | `test_journal_undo_redo_planning` | PASS | Undo and redo plan synthesis |
| `spine-merge` | `test_clean_3way_text_merge_non_overlapping` | PASS | Conflict-free 3-way text line merge |
| `spine-merge` | `test_conflicted_3way_text_merge_overlapping` | PASS | Conflict markers for overlapping file edits |
| `spine-merge` | `test_tree_merge_independent_additions` | PASS | Tree merge with disjoint additions |
| `spine-net` | `test_auth_signature_verification` | PASS | Authentication signature verification & replay check |
| `spine-net` | `test_client_server_loopback_handshake_and_ref_advertisement` | PASS | Loopback TCP client-server handshake & ref advertisement |
| `spine-net` | `test_negotiation_delta_computation` | PASS | `WANT`/`HAVE` DAG walk and minimal object delta calculation |
| `spine-net` | `test_parse_spine_url` | PASS | `spine://` URL parsing and validation |
| `spine-object-store` | `test_filesystem_object_store_corruption_detection` | PASS | Loose object bit-rot detection |
| `spine-object-store` | `test_filesystem_object_store_roundtrip` | PASS | Loose object write/read with Zstd compression |
| `spine-object-store` | `test_gc_reachability_and_grace_period` | PASS | GC reachability closure and 14-day grace period |
| `spine-object-store` | `test_pack_and_index_roundtrip_with_fanout` | PASS | `.spk` and `.spidx` 256-bucket fanout read/write |
| `spine-object-store` | `test_repack_loose_to_pack_and_composite_read` | PASS | Repack loose to pack and transparent reading |
| `spine-object-store` | `test_repo_lock_protection` | PASS | Cooperative `.spine/LOCK` acquisition and release |
| `spine-object-store` | `test_stale_lock_recovery` | PASS | Dead process PID detection and stale lock recovery |
| `spine-protocol` | `test_frame_header_roundtrip` | PASS | 20-byte binary frame header serialization |
| `spine-protocol` | `test_frame_payload_too_large_rejection` | PASS | 16 MB frame payload size ceiling enforcement |
| `spine-protocol` | `test_frame_serialization_and_checksum_verification` | PASS | BLAKE3 checksum verification on frame payloads |
| `spine-protocol` | `test_message_typed_roundtrips` | PASS | Typed message serialization (`Hello`, `RefAdv`, `Fetch`) |
| `spine-worktree` | *(None defined in lib.rs)* | N/A | Worktree checked via CLI integration tests |

### C. Binaries and Harness Summary
- Executable Test Binaries: 19 test runners
- Discovered Tests: 55
- Executed Tests: 55
- Passed Tests: 55 (100%)
- Failed Tests: 0
- Ignored Tests: 0
- Documentation Tests: 12 doc-test suites evaluated (0 doc-tests)

---

## 3. Plan for Batch 05 Test Expansion
During Batch 05, the following test suites will be added to verify networking security:
1. `tests/ed25519_auth_test.rs`: Valid/invalid Ed25519 signatures, unknown public keys, revoked keys, replayed nonces, cross-session challenge isolation.
2. `tests/tls_transport_test.rs`: TLS 1.3 encrypted loopback, valid/invalid/expired certificates, hostname validation, plaintext rejection outside loopback.
3. `tests/protocol_hardening_test.rs`: Malformed frame headers, oversized frames, out-of-order state transitions, invalid checksums, bounds overflows.
4. `tests/authorization_privacy_test.rs`: Public vs private repository metadata masking, anonymous read denial, role enforcement (Reader vs Contributor vs Admin).
5. `tests/lease_push_test.rs`: Lease-based force-push (`--force-with-lease`), stale lease rejection, protected branch rules.
6. `tests/object_closure_test.rs`: Push object-graph completeness validation, missing tree/blob rejection.
7. `tests/transfer_fault_injection_test.rs`: Controlled disconnections at handshake, negotiation, transfer, and CAS stages followed by `spine fsck` verification.
