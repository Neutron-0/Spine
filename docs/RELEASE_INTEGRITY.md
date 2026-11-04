# Spine Release Integrity & Cryptographic Attestation

## 1. Supply Chain Security & Build Reproducibility

Every Spine release artifact (binaries, packages, container images) is subjected to cryptographic attestation:
1. **Deterministic Binaries:** Cargo release builds are compiled with stripped debug symbols and pinned dependency manifests (`Cargo.lock`).
2. **SHA-256 Checksums:** All distributed tarballs and executables include a signed `SHA256SUMS` file.
3. **Software Bill of Materials (SBOM):** Generated in CycloneDX JSON and SPDX formats during CI builds, enumerating all third-party Rust crates and dependencies.

---

## 2. Release Verification Workflow

Developers and operations teams can verify the integrity of distributed Spine binaries before installation:

```bash
# 1. Download binary and signature
curl -LO https://releases.spine-vcs.org/v0.1.0/spine-api-linux-amd64.tar.gz
curl -LO https://releases.spine-vcs.org/v0.1.0/SHA256SUMS
curl -LO https://releases.spine-vcs.org/v0.1.0/SHA256SUMS.sig

# 2. Verify SHA-256 checksum
sha256sum --check --ignore-missing SHA256SUMS

# 3. Verify cryptographic signature via Minisign / Cosign
minisign -Vm SHA256SUMS -p spine-release.pub
```
