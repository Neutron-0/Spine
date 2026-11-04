# Webhook Security, SSRF Defense, and Signature Verification

## 1. Threat Landscape

Webhooks involve server-initiated outbound HTTP requests to user-supplied URLs. Without rigorous security defenses, this creates critical vulnerabilities:
1. **Server-Side Request Forgery (SSRF)**: Attackers submitting loopback (`127.0.0.1`), private RFC 1918 IPs, or link-local endpoints (`169.254.169.254`) to probe internal networks, databases, or cloud provider instance metadata.
2. **DNS Rebinding & TOCTOU**: Attackers providing a domain that resolves to a public IP during registration, but changes to a private IP when the delivery request is made.
3. **Payload Tampering & Spoofing**: Man-in-the-middle attackers forging or modifying delivery events.
4. **Denial of Service / Port Scanning**: Specifying internal infrastructure ports (SSH 22, Redis 6379, Postgres 5432, Elasticsearch 9200) to cause timeouts or crash internal daemons.

---

## 2. SSRF Defense Implementation (`spine-api::webhook::ssrf`)

Spine implements comprehensive defenses before any socket connection is initiated:

### 2.1 Scheme & Port Filtering
- Permitted schemes: `http` and `https` exclusively.
- Prohibited ports: Internal database and administrative ports are blocked by policy:
  `22, 23, 25, 53, 69, 111, 135, 139, 445, 2049, 2379, 2380, 3306, 5432, 6379, 9200, 9300, 11211, 27017`.

### 2.2 Strict IP Range Prohibitions
Every resolved address is validated against blocked network ranges:
- **Loopback**: `127.0.0.0/8`, `::1`
- **Cloud Metadata / Link-Local**: `169.254.0.0/16` (including `169.254.169.254`), `fe80::/10`
- **RFC 1918 Private**: `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`
- **Carrier-Grade NAT**: `100.64.0.0/10`
- **Broadcast / Unspecified**: `0.0.0.0/8`, `255.255.255.255`, `::`
- **IPv4-Mapped IPv6**: `::ffff:0:0/96` (unwrapped and validated as IPv4)
- **Multicast & Reserved**: `224.0.0.0/4`, `ff00::/8`

### 2.3 DNS Pinning to Prevent DNS Rebinding
To prevent Time-of-Check to Time-of-Use (TOCTOU) DNS rebinding attacks:
1. `validate_and_resolve` performs DNS resolution once.
2. All resolved IP addresses are verified against the SSRF policy.
3. The worker connects **directly to the validated `SocketAddr`**, passing the original hostname only in the HTTP `Host` header. This guarantees that no subsequent DNS resolution occurs at socket connection time.

---

## 3. HMAC-SHA256 Signature Verification

Every outbound webhook delivery includes the `X-Spine-Signature-256` header:
```http
POST /webhook HTTP/1.1
Host: ci.external.org
User-Agent: Spine-Hookshot/1.0
X-Spine-Event: push
X-Spine-Delivery: d4b3c2a1-0000-4000-8000-0123456789ab
X-Spine-Signature-256: sha256=9b71d224bd62f3785d96d46ad3ea3d73319bfbc2890caadae2dff72519673ca7
Content-Type: application/json
Content-Length: 142

{ ... payload ... }
```

### 3.1 Receiver Verification Example (Rust)
```rust
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;

pub fn verify_spine_webhook(secret: &str, body: &[u8], signature_header: &str) -> bool {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    let expected = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

    if signature_header.len() != expected.len() {
        return false;
    }
    signature_header.as_bytes().ct_eq(expected.as_bytes()).into()
}
```
*Note: Signature comparison must always be executed in constant time (`subtle::ConstantTimeEq`) to prevent timing side-channel attacks.*
