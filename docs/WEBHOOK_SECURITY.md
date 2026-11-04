# WEBHOOK NETWORK SECURITY & SSRF MITIGATION SPECIFICATION

**Date:** 2026-11-04  
**Batch Milestone:** Batch 14 — Full-System Integration, Security Hardening, Reliability, Recovery, and Performance  
**Auditor / Engineering Lead:** DeepMind Advanced Agentic Coding Pair Program  
**Workspace:** `d:\Vault\Spine`

---

## 1. CONNECTION-BOUNDARY SSRF DEFENSE

Spine protects internal networks and infrastructure against Server-Side Request Forgery (SSRF) at the actual socket connection boundary in `webhook/ssrf.rs` and `webhook/worker.rs`:

### Forbidden Destinations
1. **Loopback:** `127.0.0.0/8`, `::1`.
2. **RFC 1918 Private Ranges:** `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`.
3. **Cloud Metadata Endpoints:** `169.254.169.254`, `169.254.0.0/16`.
4. **Carrier-Grade NAT (CGNAT):** `100.64.0.0/10`.
5. **IPv4-Mapped IPv6:** `::ffff:127.0.0.1` (unwrapped and checked against IPv4 rules).
6. **Unique Local IPv6:** `fc00::/7`.
7. **Prohibited Ports:** Administrative & DB ports (`22`, `3306`, `5432`, `6379`, `27017`, `9200`, `2375`).

### DNS Rebinding Defense & Pinned Sockets
Unlike standard HTTP clients that resolve a hostname once for validation and again for HTTP dispatch (creating a Time-Of-Check to Time-Of-Use vulnerability), Spine:
1. Resolves DNS via `tokio::net::lookup_host`.
2. Validates all resolved `IpAddr` instances against the SSRF filter.
3. Pins the validated `SocketAddr` and opens a direct `TcpStream::connect(socket_addr)`.
4. Dispatches the raw HTTP/1.1 payload directly over that established socket.
5. **Does NOT follow HTTP redirects.** Any 301/302 response is recorded as a delivery result without redirect dispatch, eliminating redirect-based SSRF bypasses.

---

## 2. DELIVERY RELIABILITY & IDEMPOTENCY

1. **Transactional Outbox:** Events are inserted into `webhook_outbox` in the same database transaction as the triggering VCS or Forge operation.
2. **Exponential Backoff:** Failed deliveries retry with exponential backoff (`1s * 2^(attempts-1)`), up to `max_attempts` (default 5).
3. **HMAC-SHA256 Signing:** Every request contains `X-Spine-Signature-256: sha256=<hex>` computed using the webhook secret. Verification uses constant-time comparison (`subtle::ConstantTimeEq`).
4. **Bounded Buffers:** Remote HTTP response ingestion is strictly capped at 64 KiB to prevent memory exhaustion attacks from malicious endpoints.
