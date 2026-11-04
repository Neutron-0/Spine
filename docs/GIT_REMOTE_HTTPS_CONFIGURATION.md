# Spine Forge — Git Remote HTTPS Configuration Guide

## 1. Overview

In production deployments, Spine Forge repositories should always be accessed over HTTPS to safeguard Personal Access Tokens (PATs) and user credentials against eavesdropping or man-in-the-middle tampering.

---

## 2. Configuration Options

Spine Forge supports configuring the canonical public origin via environment variables or command-line arguments:

### Environment Variables:
```bash
# Sets canonical HTTPS origin for clone URLs and LFS action hrefs
export FORGE_PUBLIC_ORIGIN="https://forge.spine.build"

# Optional alternative variable
export APP_URL="https://forge.spine.build"

# Strictly enforce HTTPS, rejecting unencrypted Basic auth requests
export SPINE_REQUIRE_HTTPS="1"
```

### CLI Flag:
```powershell
spine-api.exe --bind 0.0.0.0 --port 3000 --public-origin https://forge.spine.build
```

---

## 3. Reverse Proxy Architecture

When deploying behind reverse proxies (e.g. Nginx, Caddy, Envoy, AWS ALB):
1. Terminate TLS at the reverse proxy.
2. Ensure the proxy forwards:
   - `Host: forge.spine.build`
   - `X-Forwarded-Proto: https`
   - `X-Forwarded-For: <client_ip>`
3. Spine Forge inspects `X-Forwarded-Proto` and the loopback origin check in `check_transport_security`.
4. If the request arrives over plaintext HTTP without `X-Forwarded-Proto: https` from a non-loopback host, Spine Forge returns:
   ```
   HTTP/1.1 403 Forbidden
   HTTPS is required for authenticated remote transport; plaintext Basic authentication is prohibited
   ```

---

## 4. Local Development & Test Mode

During local development and automated testing:
- Loopback addresses (`127.0.0.1`, `localhost`, `[::1]`) are exempted from the mandatory TLS requirement, allowing end-to-end integration testing over local loopback HTTP.
- Clone URLs generated in development default to `http://127.0.0.1:3000/<owner>/<repo>.git`.
