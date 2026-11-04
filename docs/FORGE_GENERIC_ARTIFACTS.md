# FORGE GENERIC ARTIFACTS REGISTRY

## 1. Overview
The Generic Artifacts system provides a versioned, namespace-organized repository for non-code artifacts such as:
- Machine learning model weights (`.onnx`, `.pt`, `.safetensors`).
- Game engine asset bundles.
- Nightly build outputs and raw binaries.
- Container images and filesystem tarballs.

---

## 2. Namespace Hierarchy

Artifacts are categorized under hierarchical namespaces:
```
<owner> / <namespace> / <artifact_name> / <version>
```
Example:
- `spine / toolchains / rust-nightly / 2026.11.04`
- `acme / vision-models / yolo-detector / v3.2.0`

---

## 3. Immutability Semantics

1. **Version Immutability**: Once an artifact version is published under a namespace, its payload and manifest ID cannot be overwritten. This guarantees deterministic build reproducibility.
2. **Metadata & Labels**: Flexible JSON metadata allows storing build IDs, git commit hashes, compiler flags, and license information alongside the version.

---

## 4. API Reference

### Create Namespace
```http
POST /api/v1/artifacts/namespaces
{
  "owner_id": "org_spine",
  "name": "toolchains",
  "description": "Pre-compiled toolchains and compilers",
  "is_public": true
}
```

### Upload Version
```http
POST /api/v1/artifacts/:namespace/:artifact/versions/:version
Body: <Raw Binary Stream>
```

### Download Version
```http
GET /api/v1/artifacts/:namespace/:artifact/versions/:version/download
```
Reconstructs and streams the artifact payload with BLAKE3 checksum validation in bounded memory.
