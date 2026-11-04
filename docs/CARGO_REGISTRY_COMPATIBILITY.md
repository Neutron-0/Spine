# CARGO REGISTRY COMPATIBILITY

## 1. Native Cargo Support

Spine Forge implements an official **Cargo Sparse Index Registry** in Rust, compliant with Cargo's HTTP sparse index protocol and binary publish wire format.

Strictly **zero** Git commands, external crates.io proxies, or legacy git-based index repos are used. Everything is served dynamically via Axum and backed by `spine-artifact`.

---

## 2. Sparse Index Layout

In accordance with RFC 2789, Spine formats package prefixes deterministically:

| Crate Name Length | Prefix Rule | Example Crate | Sparse Index Path |
|---|---|---|---|
| 1 char | `1` | `a` | `/index/1/a` |
| 2 chars | `2` | `ab` | `/index/2/ab` |
| 3 chars | `3/<first_char>` | `abc` | `/index/3/a/abc` |
| 4+ chars | `<char1><char2>/<char3><char4>` | `spine-artifact` | `/index/sp/in/spine-artifact` |

### Index Entry Schema
Each line in a crate's index file is a compact JSON object:
```json
{
  "name": "spine-test-crate",
  "vers": "0.1.0",
  "deps": [
    {
      "name": "serde",
      "req": "^1.0",
      "features": ["derive"],
      "optional": false,
      "default_features": true,
      "target": null,
      "kind": "normal"
    }
  ],
  "cksum": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
  "features": {},
  "yanked": false,
  "v": 2
}
```

---

## 3. Binary Publish Wire Protocol

When `cargo publish --registry spine` executes, Cargo transmits:
1. `u32` Little-Endian: Length of metadata JSON.
2. UTF-8 JSON bytes: Package metadata, dependencies, author, description, and features.
3. `u32` Little-Endian: Length of `.crate` tarball.
4. `.crate` bytes: Gzip-compressed tarball containing crate source files.

`CargoRegistryAdapter::parse_publish_payload`:
- Unpacks the LE length headers safely with bounds checking.
- Computes SHA-256 over `.crate` bytes for Cargo's `cksum`.
- Streams `.crate` bytes into `ArtifactService` (generating BLAKE3 chunk hashes and manifest).
- Inserts new version into `package_versions` and appends to crate index.

---

## 4. Developer Configuration

Developers configure Spine in `.cargo/config.toml`:

```toml
[registries.spine]
index = "sparse+https://forge.spine.build/index/"
credential-provider = "cargo:token"
```

Authenticate with Personal Access Token (PAT):
```bash
$ cargo login --registry spine
# Enter your Spine PAT (e.g. sp_pat_...)
```

Publish directly:
```bash
$ cargo publish --registry spine
    Updating `spine` index
   Packaging my-crate v0.1.0 (...)
   Uploading my-crate v0.1.0 (...)
   Published my-crate v0.1.0 at registry `spine`
```

Consume in projects:
```toml
[dependencies]
my-crate = { version = "0.1.0", registry = "spine" }
```
