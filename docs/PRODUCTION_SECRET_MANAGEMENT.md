# Spine Production Secret Management & Credential Security

## 1. Threat Model & Security Posture

Production deployment of Spine handles multiple classes of high-entropy secrets and credentials:
1. **Database Credentials (`DATABASE_URL`)**: Provides read/write access to all user accounts, access tokens, and repository permissions.
2. **SSH Host Private Key (`host_key_ed25519`)**: Proves server identity for native Git-over-SSH; compromise allows Man-in-the-Middle (MitM) attacks.
3. **Session & Token Signing Keys**: Signs user authentication tokens and CSRF cookies.
4. **Webhook Signing Secrets**: Verifies authenticity of outbound webhook deliveries.
5. **CI Runner Registration Tokens**: Authorizes remote compute runners to connect and receive build payloads.

---

## 2. Mandatory Rules for Secret Handling

1. **Zero Hardcoded Secrets**:
   - Source code, Dockerfiles, and committed configuration files must never contain default or production passwords, API tokens, or private keys.
   - All secret material must be supplied at runtime via environment variables or secret files mounted into `/etc/spine/secrets/`.
2. **One-Time Administrator Bootstrap**:
   - The root administrator cannot be created with a default static password like `admin` or `password123`.
   - Administrators must invoke `spine-api bootstrap-admin --username <name> --email <email>`.
   - The CLI generates a 32-character cryptographically random password using `rand`, hashes it via Argon2id, prints it to stdout once, and records an immutable audit event (`system.bootstrap_admin`). Subsequent executions are locked out.
3. **Strict Filesystem Permissions**:
   - Secret files and host keys must be permission-restricted:
     - Directory: `chmod 700 /var/lib/spine/storage/ssh`
     - Private key: `chmod 600 /var/lib/spine/storage/ssh/host_key_ed25519`
     - Environment file: `chmod 600 /etc/spine/spine-api.env`
     - File ownership must strictly belong to `spine:spine`.
4. **Secret Injection in Container Environments**:
   - Do not pass sensitive secrets in command-line arguments (visible via `ps aux`).
   - In Kubernetes, mount secrets as projected volumes (`/etc/spine/secrets`).
   - In Docker Compose, reference an external `.env` file or Docker secrets mechanism.
