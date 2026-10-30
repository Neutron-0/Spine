# SPINE FORGE CI/CD ARCHITECTURE & SANDBOX RUNNER

## 1. System Topology
The CI/CD subsystem executes untrusted repository code in strictly isolated, ephemeral environments:

```
+-------------------+      Queue Job      +--------------------+
|  Spine API / Hook | -----------------> |    Spine Runner    |
+-------------------+                    | (Host Controller)  |
                                         +--------------------+
                                                   | Spawns
                                                   v
                                         +--------------------+
                                         | Isolated Container |
                                         | - Read-only root   |
                                         | - No host mounts   |
                                         | - Ephemeral disk   |
                                         | - Network sandbox  |
                                         +--------------------+
```

## 2. Security Boundaries
1. **Never Privileged:** Containers run as unprivileged users with stripped Linux capabilities (or Windows AppContainer sandbox).
2. **Ephemeral Lifecycles:** Each job runs in a pristine sandbox discarded immediately upon completion.
3. **No Host Access:** Host storage containing `.spine/` repositories is never mounted inside runner containers; repository sources are cloned via a scoped, temporary checkout token.
4. **Secret Masking:** Environment variables containing repository secrets are automatically masked in real-time execution logs.
