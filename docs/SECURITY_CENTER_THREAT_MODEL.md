# Security Center & Supply Chain Defense Threat Model

Date: 2026-11-04  
System: SPINE Security Center  

---

## 1. Threat Vectors Addressed

1. **Accidental Credential Exposure**: Developers committing API keys, PATs, cloud access credentials, or private keys to repository history.
2. **Plaintext Secret Leakage in Audit Trails**: Security monitoring tools logging or returning discovered credentials in plaintext within alerts, database rows, or notification webhooks.
3. **Known Vulnerable Dependencies (CVEs)**: Inclusion of compromised, vulnerable, or deprecated third-party packages.
4. **Supply Chain Non-Compliance**: Lack of machine-readable Software Bill of Materials (SBOM) for regulatory audit and compliance frameworks (Executive Order 14028, NTIA minimum elements).

---

## 2. Defensive Controls

### 2.1 Safe Credential Fingerprinting
To prevent the Security Center from becoming an exfiltration vector:
- Discovered tokens are parsed to extract high-confidence matches.
- A cryptographic SHA-256 digest is computed from the raw secret.
- The raw secret is **never stored** in `secret_scan_findings`, **never included** in REST API responses, and **never printed** in server log streams.
- The system renders a masked safe fingerprint (e.g. `sp_p...****...9f01 (sha256:d8a2bc41)`), allowing developers to identify which credential was leaked without exposing the live credential itself.

### 2.2 Automated Cargo Dependency Auditing
- The Security Center inspects repository build manifests (`Cargo.lock` and `Cargo.toml`).
- Extracted dependency entries are checked against the `security_advisories` catalog.
- If a package version falls within a vulnerable range (`<0.2.14`, `<=0.9.5`), an active `DependencyVulnerability` alert is generated with recommended patched version targets.

### 2.3 Standards-Compliant SBOM Export
- **SPDX 2.3 (JSON)**: Produces standard packages, licenses, and suppliers compatible with SPDX auditing tools.
- **CycloneDX 1.5 (JSON)**: Produces bill-of-materials components with Package URLs (`pkg:cargo/name@version`).
- Endpoints enforce repository read access control.
