# MIGRATION SECURITY, ARCHIVE HARDENING, AND USER ISOLATION

**Project:** Spine  
**Batch Milestone:** Batch 15 — Native Git Interoperability, Repository Import/Export, Platform Migration, and Ecosystem Compatibility  
**Author:** DeepMind Advanced Agentic Coding Pair Program  
**Workspace:** `d:\Vault\Spine`

---

## 1. THREAT MODEL

Migrating code and metadata from foreign sources exposes hosting infrastructure to adversarial archives, privilege escalation attempts, and remote code execution exploits.

The Spine Migration security model explicitly defends against:

1. **Path Traversal & Zip Slip:** Malicious archive entries containing `../`, absolute paths (`/etc/passwd`, `C:\Windows\...`), or embedded drive letters attempting to write outside target destination directories.
2. **Windows Reserved Device Name Exploits:** File names such as `CON`, `PRN`, `AUX`, `NUL`, `COM1-9`, `LPT1-9` (even with extensions like `CON.txt` or `nul.json`) causing OS denial-of-service, blue screens, or file system hangs on Windows hosts.
3. **Decompression Bombs (Zip Bombs):** Highly compressed tiny archives expanding to terabytes of data or millions of tiny inodes to exhaust disk and memory.
4. **Untrusted CI/CD Execution:** Workflows ported from GitHub Actions or GitLab CI executing arbitrary commands or attempting token theft on Spine runner infrastructure.
5. **Privilege Escalation via Identity Attribution:** External contributors acquiring unauthorized administrative permissions on Spine organizations or repositories through metadata impersonation.
6. **Hardcoded Secrets & Credential Leakage:** API keys, tokens, and private keys embedded in commit history or platform issue threads.

---

## 2. HARDENED ARCHIVE EXTRACTION (`archive_security.rs`)

Spine enforces multiple independent defense layers during archive extraction:

```rust
pub fn validate_and_sanitize_archive_path(
    destination_root: &Path,
    entry_path: &Path,
) -> Result<PathBuf, MigrationError>
```

### 2.1 Traversal & Boundary Checks
- Rejects any path component containing `..` (parent directory traversal).
- Strips leading `/` and `\` to normalize relative paths.
- Verifies that the canonicalized path starts strictly with the canonical destination root.

### 2.2 Windows Reserved Device Name Blocker
- Inspects every file and directory segment stem (case-insensitively).
- Explicitly rejects:
  - `CON`, `PRN`, `AUX`, `NUL`
  - `COM1` through `COM9`
  - `LPT1` through `LPT9`
- Rejection occurs regardless of extension (`aux.rs`, `NUL.tar`, `prn.txt` are all blocked).

### 2.3 Decompression Thresholds
- **Maximum File Count:** Enforces a maximum limit (default: 1,000,000 files).
- **Maximum Uncompressed Bytes:** Enforces a per-archive extraction limit (default: 10 GB).
- **Compression Ratio Ceiling:** If uncompressed bytes exceed 100x the compressed stream bytes, extraction immediately halts with `DecompressionBombDetected`.

---

## 3. SAFE ATTRIBUTION & GHOST USER ENFORCEMENT

To prevent account hijacking or unauthorized privilege inheritance:

1. **Zero Admin Grant:** Imported users from external platforms are **never** granted repository `admin`, `write`, or organization `owner` roles during migration.
2. **Ghost User Policy (Default):**
   - External users who do not exist on the Spine Forge instance are mapped to safe ghost accounts: `ghost-<username>`.
   - Email addresses are mapped to non-routable aliases: `<username>@migrated.ghost.spine`.
3. **Email Match Policy (Opt-In):**
   - If an existing registered Spine Forge user possesses a verified email matching the external commit or issue author, the activity is linked to their profile as standard contributor activity without role alteration.

---

## 4. FAIL-CLOSED WORKFLOW DEACTIVATION

- Any workflow definition extracted from `.github/workflows/` or `.gitlab-ci.yml` is inspected and sanitized.
- The workflow record is saved into Spine Forge with `is_active: false`.
- Runners will refuse to execute any job from the migrated workflow until a repository administrator logs into the Forge UI or CLI and explicitly activates it.
