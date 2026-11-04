# PACKAGE REGISTRY ARCHITECTURE

## 1. Overview
Spine Forge includes a multi-ecosystem package registry architecture backed by the unified `spine-artifact` storage engine.

While designed to support npm, PyPI, OCI, and Maven, Batch 11 implements full first-class compatibility for the **Rust / Cargo** ecosystem via `CargoRegistryAdapter`.

---

## 2. Universal Package Abstraction

Every package registry in Spine shares a common core model:
```
+-----------------------------------------------------------+
|                          Package                          |
| - id: String                                              |
| - ecosystem: "cargo" | "npm" | "pypi" | ...              |
| - name: String                                            |
| - owner_id: String                                        |
+-----------------------------------------------------------+
                             │
                             ▼
+-----------------------------------------------------------+
|                      PackageVersion                       |
| - id: String                                              |
| - version: SemVer ("0.1.0")                               |
| - manifest_id: String (BLAKE3 ArtifactManifest)           |
| - checksum_sha256: String                                 |
| - size_bytes: u64                                         |
| - yanked: bool                                            |
| - metadata_json: String                                   |
+-----------------------------------------------------------+
```

---

## 3. Guarantees & Constraints

1. **Version Immutability**:
   Once version `X.Y.Z` of package `P` is published, re-publishing or overwriting `X.Y.Z` is strictly rejected with HTTP 409 Conflict.
2. **Yanking Semantics**:
   - Yanking prevents the package version from satisfying new dependency resolution constraints.
   - Existing lockfiles with the exact version can still download the `.crate` tarball, ensuring existing builds do not break.
   - Crate owners can un-yank if a version was yanked by mistake.
3. **Bounded Memory Ingestion**:
   During upload/publish, packages are streamed and chunked in 4 MiB memory bounds, enabling publication of large crates and embedded binary distributions without memory exhaustion.
