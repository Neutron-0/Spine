# Spine Production Network Architecture & Ingress Topology

## 1. Perimeter & Demilitarized Zone (DMZ) Layout

A production Spine deployment utilizes a layered network architecture that isolates untrusted public clients, reverse proxy terminators, internal control-plane APIs, and database nodes:

```
                  ┌─────────────────────────────────────────┐
                  │          Internet / Developer Clients   │
                  └───────────────────┬─────────────────────┘
                                      │
                         HTTPS:443    │    SSH:22 / SSH:2222
                                      ▼
                  ┌─────────────────────────────────────────┐
                  │    Ingress / Load Balancer / Proxy      │
                  │       (Nginx / Traefik / Envoy)         │
                  └─────────┬─────────────────────┬─────────┘
                 Proxy HTTP │                     │ Stream TCP
                 Port 3000  │                     │ Port 2222
                            ▼                     ▼
                  ┌─────────────────────────────────────────┐
                  │         Spine Forge API & SSH           │
                  │             (spine-api)                 │
                  └─────────┬─────────────────────┬─────────┘
                            │                     │
                gRPC / RPC  │                     │ PostgreSQL:5432
                Port 8400   │                     │
                            ▼                     ▼
                  ┌──────────────────┐   ┌──────────────────┐
                  │Spine Coordinator │   │ PostgreSQL 16    │
                  └─────────┬────────┘   │ Primary Cluster  │
                            │RPC:9101    └──────────────────┘
                            ▼
                  ┌──────────────────┐
                  │Spine Storage Node│
                  └──────────────────┘
```

---

## 2. Port Allocation & Protocol Specification

| Port | Service | Protocol | Access Scope | Description |
|---|---|---|---|---|
| **443** | Ingress | HTTPS | Public / WAN | TLS termination for Web UI, Git Smart HTTP, REST API, Git LFS |
| **22 / 2222** | Ingress / API | SSH (TCP) | Public / WAN | Native Git-over-SSH transport and LFS authenticate command |
| **3000** | `spine-api` | HTTP/1.1, HTTP/2 | Internal Only | Core Forge API, Webhook receiver, Health endpoints |
| **8400** | `coordinator` | Spine RPC (TCP) | Internal Only | Cluster topology management, lease negotiation, heartbeats |
| **9101-9103**| `storage-node`| Spine RPC (TCP) | Internal Only | Object streaming, packfile replication, journal commits |
| **5432** | PostgreSQL | PostgreSQL (TCP)| Internal Only | Relational metadata persistence |

---

## 3. Ingress Reverse Proxy Configuration (Nginx Reference)

```nginx
# Web UI, Git Smart HTTP, REST API, Git LFS
server {
    listen 443 ssl http2;
    server_name forge.example.com;

    ssl_certificate /etc/letsencrypt/live/forge.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/forge.example.com/privkey.pem;

    client_max_body_size 10G; # Required for large LFS uploads and packfiles

    location / {
        proxy_pass http://127.0.0.1:3000;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto https;
        proxy_buffering off;
        proxy_request_buffering off; # Streaming Git push / LFS
    }
}

# SSH Passthrough (Nginx stream block in nginx.conf)
stream {
    server {
        listen 22;
        proxy_pass 127.0.0.1:2222;
        proxy_timeout 1h;
    }
}
```
