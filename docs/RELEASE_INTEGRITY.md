# Spine Release Integrity, Cryptographic Attestation & Trust Anchor Architecture

## 1. Supply Chain Security & Build Reproducibility

Every Spine release candidate and distributed artifact is subjected to cryptographic attestation:
1. **Deterministic Binaries:** Cargo release builds are compiled with stripped debug symbols and pinned dependency manifests (`Cargo.lock`).
2. **Cryptographic Signatures (Ed25519):** All distributed tarballs and executables include a signed `SHA256SUMS.sig` and binary-level signatures.
3. **Authenticated Provenance (`PROVENANCE.json`):** Bound to the git commit hash, repository URL, build timestamp, and SHA-256 digests of all released binaries, signed with Ed25519.
4. **Software Bill of Materials (SBOM):** Generated in CycloneDX JSON and SPDX formats during CI builds, enumerating all third-party Rust crates and dependencies.

---

## 2. Trust Anchor Architecture & Independent Authentication

### The Bundled Public Key Fallacy
In supply chain security, a public key bundled inside an untrusted download archive **MUST NEVER** be accepted as a trust anchor without independent authentication. If an attacker tampers with release binaries in transit or compromises an untrusted distribution mirror, they can trivially replace the binary, recalculate the checksums, re-sign with an attacker-generated key, and replace the bundled public key. Blindly trusting the bundled key would cause verification to pass erroneously.

### Independent Trust Anchor Distribution Channels
Spine strictly requires operators to obtain or verify the Ed25519 public key through one of three independently authenticated channels:

1. **Repository Pinned Trust Anchor:**
   - Path: `deploy/security/spine-release-signing.pub`
   - Bound directly to git repository history and protected branches.
2. **Out-of-Band DNS TXT Attestation:**
   - Domain: `_spine-release-key.spine.dev`
   - Published Record: `v=spine1; alg=ed25519; fp=dada4ec5ae0317e05f62915fa8b330364d95b542095f9e8a719bf8beae09fd02; key=4ac344ceb9efd42cf898aa55c5df05b22b07e05ec68661eb5271eb53dfbbef77`
   - Operators can verify the fingerprint over DNSSEC before importing.
3. **Signed Git Release Tag / Maintainer GPG Web of Trust:**
   - Every official release tag (e.g. `v0.1.0-rc3`) is cryptographically signed with the maintainer's hardware security key (YubiKey / GPG) verified against public key registries.
4. **Out-of-Band Enterprise Configuration Management:**
   - Pinned into hardened infrastructure templates (Ansible, Puppet, Terraform) deployed to `/etc/spine/keys/spine-release-signing.pub`.

---

## 3. Release Verification Procedure

Operators must verify the complete artifact suite before deployment:

```bash
# 1. Download release directory or tarball
mkdir -p dist/spine-v0.1.0-rc3
curl -LO https://releases.spine-vcs.org/v0.1.0-rc3/spine-api-linux-amd64
curl -LO https://releases.spine-vcs.org/v0.1.0-rc3/spine-coordinator-linux-amd64
curl -LO https://releases.spine-vcs.org/v0.1.0-rc3/spine-storage-node-linux-amd64
curl -LO https://releases.spine-vcs.org/v0.1.0-rc3/SHA256SUMS
curl -LO https://releases.spine-vcs.org/v0.1.0-rc3/SHA256SUMS.sig
curl -LO https://releases.spine-vcs.org/v0.1.0-rc3/PROVENANCE.json
curl -LO https://releases.spine-vcs.org/v0.1.0-rc3/PROVENANCE.json.sig

# 2. Verify release using the independent trust anchor
python scripts/verify_release_signatures.py \
  --artifacts-dir dist/spine-v0.1.0-rc3 \
  --trusted-key-file deploy/security/spine-release-signing.pub
```

### Verification Guarantees
- Rejects any release if `deploy/security/spine-release-signing.pub` does not match the signature.
- Rejects releases if a bundled key is substituted.
- Rejects releases if any binary binary bit has been flipped (checksum mismatch).
- Rejects releases if `PROVENANCE.json` has been altered.
- Rejects releases if the git commit hash in provenance does not match the signed manifest.
