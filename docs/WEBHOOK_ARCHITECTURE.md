# Spine Webhook Subsystem Architecture

## 1. Overview and Design Principles

The **Spine Webhook Subsystem** provides reliable, at-least-once event delivery from Spine Forge to external consumer services (such as chat notifications, external CI/CD systems, deployment managers, and auditing platforms).

Key architectural principles:
- **Transactional Outbox Pattern**: Webhook events are enqueued in the database as part of the same transaction or workflow event that produced them.
- **Asynchronous Decoupled Dispatcher**: Background outbox workers claim pending batches, deliver payloads, and manage retries without delaying Forge user operations.
- **HMAC-SHA256 Signatures**: Payloads are signed with secret keys, providing authenticity and tamper-evidence.
- **Exponential Backoff**: Transient network or server failures are retried up to 5 times with exponential backoff.
- **Comprehensive Delivery Auditing**: Full request headers, payload summaries, HTTP status codes, latency in milliseconds, and response bodies are captured for developer inspection.

---

## 2. Webhook Event Delivery Flow

```
[Repository or Organization Event]
          |
          v
[WebhookService::dispatch_event]
          | - Finds matching active subscriptions
          | - Computes HMAC-SHA256 signature
          | - Enqueues WebhookOutboxItem (status: PENDING)
          v
[Transactional Outbox Table]
          |
          | Worker claims batch (limit N, scheduled <= now)
          v
[WebhookOutboxWorker]
          | - SSRF validation & DNS pinning (resolved SocketAddr)
          | - Dispatches HTTP POST with headers:
          |     X-Spine-Event, X-Spine-Delivery, X-Spine-Signature-256
          | - Reads response (up to 64 KiB)
          +-----------------------+-----------------------+
                                  |                       |
                             (2xx Success)           (4xx/5xx/Timeout)
                                  |                       |
                                  v                       v
                          Mark DELIVERED         Increment attempts
                          Record SUCCESS log     If attempts >= 5: FAILED
                                                 Else: Exponential backoff
```

---

## 3. Supported Events

| Event Type | Trigger Description |
|---|---|
| `push` | Commits pushed to a repository branch |
| `pull_request` | PR opened, edited, closed, or merged |
| `pull_request_review` | Review submitted or state changed |
| `issues` | Issue created, updated, or closed |
| `workflow_run` | CI workflow run queued or completed |
| `release` | Release published or draft created |
| `package` | Package version published or yanked |
| `*` | Wildcard: all events |
