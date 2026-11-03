# Spine Forge: Identity and Authentication Architecture

## 1. Authentication Primitives

Spine Forge implements an uncompromising cryptographic identity and session system designed to safeguard developer accounts, API tokens, and command-line interactions.

### 1.1 Password Security & Argon2id Hashing
- **Complexity Policy**: Passwords must contain a minimum of 8 characters (up to 128) and satisfy three character classes: uppercase, lowercase, and numeric digits.
- **Algorithm**: Argon2id (`argon2::Argon2::default()`), resistant to both GPU/ASIC attacks and side-channel cache attacks.
- **Salts**: Cryptographically random 16-byte salts generated via `rand::rngs::OsRng`.
- **Verification**: Constant-time verification prevents timing attacks.

### 1.2 Session Management
- **Token Generation**: 32 cryptographically secure random bytes generated via OS CSPRNG, formatted as `sp_sess_<64_hex_chars>`.
- **Database Storage**: Raw tokens are never persisted. Only the BLAKE3 hash of the token (`hash_session_token`) is stored in the `user_sessions` table.
- **Lifetime**: Default TTL is 30 days. Active sessions track IP address, user-agent, creation time, and expiration.
- **Revocation**: Single-session revocation (`DELETE /api/v1/users/me/sessions/:id`) and global revocation across all devices (`POST /api/v1/auth/password/change`).

### 1.3 Personal Access Tokens (PAT)
- **Token Format**: `sp_pat_<64_hex_chars>`.
- **Hashing**: Indexed by BLAKE3 hash. Fast prefix (`sp_pat_abcd...`) enables user recognition without revealing token entropy.
- **Scopes**: Granular capability control:
  - `repo:read`: Read repository files, trees, branches, and commit history.
  - `repo:write`: Push commits, create tags, create repositories.
  - `admin:all`: Full administrative control over user resources.
- **Last Used Tracking**: Timestamp is updated on usage to surface dormant keys.

### 1.4 Rate Limiting
- **Mechanism**: In-memory, thread-safe sliding window rate limiter (`RateLimiter`).
- **Policy**: Protects sensitive endpoints (login, registration, password reset) by tracking attempt timestamps per client IP.
- **Window**: Configurable threshold (default: 10 attempts per minute) prevents brute-force credential stuffing.

### 1.5 Password Reset and Email Verification
- **One-time Tokens**: 32-byte CSPRNG hex tokens stored hashed.
- **Single Use Consumption**: Database atomic `DELETE ... RETURNING user_id` guarantees tokens cannot be replayed or reused.
- **Expiration**: Email verification valid for 24 hours; password reset valid for 1 hour.
