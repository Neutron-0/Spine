# Linux Service and Systemd Unit Verification Report

**Milestone:** Spine Batch 20 — Final Deployment Readiness Closure  
**Date:** 2026-11-05  
**Platform:** Ubuntu 24.04 LTS on Linux 6.6.87.2-microsoft-standard-WSL2 (PID 1 systemd 259)  
**Units Tested:**  
- `/etc/systemd/system/spine-api.service`
- `/etc/systemd/system/spine-coordinator.service`
- `/etc/systemd/system/spine-storage-node.service`

---

## 1. Executive Summary

All three Spine systemd service units were installed into the live Linux systemd manager and verified using `systemd-analyze verify`.

The syntax, security parameters, sandboxing paths, restart policies, and unit dependency trees were validated with **zero parse errors, zero unresolved directives, and exit code 0**.

---

## 2. Unit Verification Details

```bash
# Verification Command Executed:
systemd-analyze verify \
  /etc/systemd/system/spine-api.service \
  /etc/systemd/system/spine-coordinator.service \
  /etc/systemd/system/spine-storage-node.service

# Return Code: 0 (No syntax, dependency, or path errors)
```

### Verified Sandboxing Directives
Across all three daemon services:
- **`NoNewPrivileges=true`**: Prevents child processes from gaining elevated capabilities (e.g. via setuid binaries).
- **`ProtectSystem=strict`**: Mounts the entire OS filesystem (`/usr`, `/boot`, `/etc`) read-only for the service process tree.
- **`ProtectHome=true`**: Makes `/home`, `/root`, and `/run/user` completely inaccessible and empty.
- **`PrivateTmp=true`**: Creates an isolated per-service mount namespace for `/tmp` and `/var/tmp`.
- **`PrivateDevices=true`**: Restricts physical device nodes, preventing direct hardware access.
- **`ProtectKernelTunables=true`**: Disallows writing to `/proc/sys`, `/sys`, etc.
- **`ProtectKernelModules=true`**: Explicitly blocks loading or unloading kernel modules.
- **`ProtectControlGroups=true`**: Makes cgroups read-only to prevent container escapes or resource tampering.
- **`RestrictRealtime=true`**: Blocks real-time process priority elevation.
- **`RestrictSUIDSGID=true`**: Prohibits creation of SUID/SGID bits.
- **`ReadWritePaths=/var/lib/spine`**: Confines all disk writes strictly to Spine's dedicated application state and object store directory.

---

## 3. Dependency & Ordering Graph

```text
multi-user.target
 └── spine-api.service
      ├── Wants / After: postgresql.service
      └── Wants / After: spine-coordinator.service
           └── After: network.target

multi-user.target
 └── spine-storage-node.service
      └── Wants / After: spine-coordinator.service
           └── After: network.target
```

---

## 4. Operational Recommendations

1. Ensure the system user `spine` and group `spine` are provisioned prior to unit activation:
   ```bash
   useradd -r -s /bin/false -d /var/lib/spine spine
   mkdir -p /var/lib/spine/storage /etc/spine
   chown -R spine:spine /var/lib/spine /etc/spine
   chmod 750 /var/lib/spine
   ```
2. Enable and start units in topological order:
   ```bash
   systemctl daemon-reload
   systemctl enable --now spine-coordinator.service
   systemctl enable --now spine-storage-node.service
   systemctl enable --now spine-api.service
   ```
