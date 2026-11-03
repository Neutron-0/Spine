# CLI Credential Security Specification

## 1. Zero Plaintext Exposure Guarantee

The Spine command-line interface adheres strictly to a zero-plaintext credential policy:
1. **No Configuration Storage**: Personal Access Tokens (PATs) and session secrets are **never** written to `.spine/spine.toml`, `~/.spicerc`, or any unencrypted text file.
2. **Strict Terminal Masking**: When credentials or session status are displayed in the terminal, tokens are strictly redacted:
   - For example: `sp_pat_9a8b...****...1234`
   - Tokens shorter than 12 characters are masked entirely (`********`).
3. **Audit Redaction**: Log files and debug traces automatically redact any argument or payload matching the token prefix pattern (`sp_pat_`).

## 2. OS-Native Credential Storage Integration

On Windows, Spine utilizes the Windows Credential Manager via the native Win32 `advapi32.dll` APIs (`CredWriteW`, `CredReadW`, `CredDeleteW`, `CredFree`):
- **Credential Type**: `CRED_TYPE_GENERIC`.
- **Target Name Format**: `spine:forge:<host>`.
- **Persistence**: `CRED_PERSIST_LOCAL_MACHINE` or `CRED_PERSIST_ENTERPRISE`.
- **Security Boundary**: Protected by the Windows Data Protection API (DPAPI) and tied to the user's logged-in Windows security context.

## 3. Headless Fallback Storage

For containerized environments, Linux/macOS, or headless CI runners where the Windows Credential Manager is unavailable:
- Spine falls back to `SecureFallbackStorage`.
- Stored in `~/.spine/credentials/secure_keyring.json` with restricted filesystem permissions (`0600` on Unix; explicit ACLs on Windows).
- Encapsulated within `OsCredentialManager` behind a unified trait interface.
