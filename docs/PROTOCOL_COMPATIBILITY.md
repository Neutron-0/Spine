# Spine Protocol Compatibility and Evolution Specification

## 1. Compatibility Principles
The Spine Wire Protocol (`SPWP`) is engineered to evolve over decades without breaking backward compatibility or requiring lockstep client/server upgrades.

Key principles:
1. **Explicit Version Negotiation:** The initial `HELLO` / `HELLO_ACK` handshake explicitly exchanges the highest supported protocol versions. Both parties agree on the minimum common version $\min(V_{client}, V_{server})$.
2. **Capability Negotiation Bitflags:** Specific features (compression algorithms, auth schemes, delta algorithms) are negotiated dynamically via string sets in `HELLO`.
3. **Strict Forward Ignoring of Unknown Flags:** Undefined bits in `Frame Flags` or unmapped fields in JSON payloads must be safely ignored by older receivers.
4. **Stable Binary Framing:** The 20-byte fixed header format is immutable for version 1. Any change to the header layout requires bumping the magic or major protocol version.

---

## 2. Version Lifecycle

| Version | Status | Introduced | Features |
|---|---|---|---|
| **SPWP v1** | Current / Standard | Batch 04 (2026-10) | 20-byte binary framing, Ed25519 challenge-response auth, WANT/HAVE graph traversal, direct `.spk` streaming, atomic CAS push, BLAKE3 checksums. |
| **SPWP v2** | Planned (Future) | Batch 06+ | Multiplexed QUIC datagrams, streaming chunk deduplication, partial clones with blob filters. |

---

## 3. Capability Registry (v1)

| Capability String | Description | Required / Optional |
|---|---|---|
| `pack_zstd` | Supports Zstandard-compressed pack records in `.spk` streams | Required |
| `ed25519_auth` | Supports Ed25519 public-key signature challenge-response | Required |
| `cas_push` | Supports atomic Compare-and-Swap reference update requests | Required |
| `progress_reports` | Emits and parses `0x0052 PACK_PROGRESS` telemetry | Optional |
| `sha256_compat` | Capable of negotiating SHA-256 repositories | Optional |

If a client requires a capability that the server does not advertise in `HELLO_ACK`, the client aborts the connection with a descriptive message explaining the missing feature.
