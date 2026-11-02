# Spine VCS — Security Dependencies & Supply Chain Audit (Batch 05)

## 1. Overview and Security Baseline

Spine enforces a strict supply-chain and memory-safety policy. Every security-critical dependency in the networking and cryptographic layers has been audited for memory safety, side-channel resistance, cryptographic hygiene, and minimal attack surface.

| Crate | Version | Role in Spine | Key Features / Justification |
|---|---|---|---|
| `ed25519-dalek` | `2.2.0` | Asymmetric digital signatures | Pure Rust, constant-time arithmetic, `zeroize` on drop, side-channel immune |
| `rustls` | `0.23.46` | TLS 1.3 protocol engine | Pure Rust, no OpenSSL dependency, memory-safe, no buffer overflows |
| `tokio-rustls` | `0.26.6` | Asynchronous TLS streams | Seamless integration with Tokio async event loop |
| `rustls-pki-types` | `1.15.1` | Type-safe X.509 abstractions | Zero-cost newtypes for DER certificates and server names |
| `ring` | `0.17.14` | Cryptographic primitives for rustls | Constant-time curve operations, assembly optimizations, audited |
| `rcgen` | `0.13.2` | Ephemeral X.509 cert generation | Self-signed development certificates for zero-config TLS testing |
| `blake3` | `1.5` | Content addressing & frame checksums | 256-bit secure tree hashing, domain-separated digests, SIMD acceleration |
| `rand` / `rand_chacha`| `0.8.8` | Cryptographic nonce generation | Backed by operating system CSPRNG (`OsRng`) |
| `zeroize` | `1.9.1` | Memory zeroization | Securely erases private key seeds and secrets from RAM upon drop |

---

## 2. Supply Chain Security Controls

1. **No External C/C++ OpenSSL Binding**:
   Spine completely avoids `libssl`, `libcrypto`, or `openssl-sys`. All TLS operations run in memory-safe Rust with `rustls` and `ring`, completely mitigating historic vulnerabilities like Heartbleed, buffer overreads, and uninitialized memory disclosures.

2. **Zeroize Protection**:
   `SigningKey` seeds and intermediate key states implement `Zeroize` and `ZeroizeOnDrop`, ensuring that private key bytes are scrubbed from process memory immediately after key destruction.

3. **Pinned Minimum Supported Rust Version (MSRV)**:
   All dependencies compile cleanly on stable Rust 2021 (Rust 1.97+).

4. **Cryptographic Domain Separation**:
   All hash and signature operations prefix their inputs with distinct domain tags (`b"SPWP-AUTH-v1\0"`, `b"SPWP-KEY-v1\0"`, `b"SPINE-COMMIT-v2\0"`, etc.), mathematically preventing cross-protocol type-confusion attacks.
