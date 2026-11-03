# Spine Forge: Native CLI Authentication and Operations

## 1. CLI Commands Specification

The Spine CLI integrates natively with Spine Forge, allowing developers to authenticate, manage credentials, and provision remote repositories directly from the terminal without external browser dependencies.

### 1.1 `spine auth login`
Authenticates against a Spine Forge host instance:
```bash
# Interactive prompt (prompts for username/email and password)
spine auth login --host http://127.0.0.1:3000

# Direct authentication using Personal Access Token (PAT)
spine auth login --host http://127.0.0.1:3000 --token sp_pat_9a8b7c6d...
```

### 1.2 `spine auth status`
Queries the configured Forge instances and verifies active session validity:
```bash
spine auth status
```
Output:
```text
=== Spine Forge Authentication Status ===
  Host:     http://127.0.0.1:3000
  User:     alice
  Status:   ACTIVE
```

### 1.3 `spine auth logout`
Revokes the remote session and removes credentials from disk:
```bash
spine auth logout --host http://127.0.0.1:3000
```

### 1.4 `spine repo create`
Provisions a new repository on the remote Spine Forge instance:
```bash
# Create a public repository with default ThreeCopies durability
spine repo create my-awesome-project --description "Rust distributed engine"

# Create a private repository with TwoCopies durability
spine repo create classified-tool --private --durability TwoCopies
```
Output:
```text
=== Repository Created Successfully ===
  Repository:   alice/classified-tool
  Visibility:   Private
  Status:       PENDING
  Clone URL:    spine://127.0.0.1:4434/alice/classified-tool
```

## 2. Credentials Storage

Credentials are stored in TOML format at `~/.spine/credentials.toml` (or `%USERPROFILE%\.spine\credentials.toml` on Windows):
```toml
[hosts."http://127.0.0.1:3000"]
host = "http://127.0.0.1:3000"
username = "alice"
token = "sp_sess_4f89..."
expires_at_nanos = 1775172000000000000
```

## 3. Zero-Dependency Async HTTP Client

To keep `spine-cli` minimal, fast, and secure without compiling 80+ external dependencies, CLI networking uses a custom ~60-line asynchronous HTTP/1.1 client over `tokio::net::TcpStream`:
- Implements standard HTTP/1.1 request formatting with `Host`, `Authorization: Bearer <token>`, `Content-Length`, and `Content-Type: application/json`.
- Parses HTTP status line, headers, and body payloads with zero external library overhead.
