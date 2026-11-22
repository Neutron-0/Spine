#!/usr/bin/env python3
"""
Spine Release Artifact Signing Script
Cryptographically signs release binaries and SHA256SUMS manifest using Ed25519.
Generates detached signatures (.sig) and release-provenance.json.
Private keys are never printed, logged, or checked into version control.
"""

import argparse
import datetime
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

try:
    from cryptography.hazmat.primitives.asymmetric import ed25519
    from cryptography.hazmat.primitives import serialization
    from cryptography.exceptions import InvalidSignature
except ImportError:
    sys.stderr.write("ERROR: Python 'cryptography' library is required. Install with: pip install cryptography\n")
    sys.exit(2)


def compute_sha256(filepath: Path) -> str:
    """Computes SHA-256 hash of a file."""
    h = hashlib.sha256()
    with open(filepath, "rb") as f:
        while chunk := f.read(65536):
            h.update(chunk)
    return h.hexdigest()


def get_git_commit(cwd: Path) -> str:
    """Gets current git commit hash if in a git repo."""
    try:
        out = subprocess.check_output(
            ["git", "rev-parse", "HEAD"],
            cwd=str(cwd),
            stderr=subprocess.DEVNULL,
            text=True,
        ).strip()
        return out
    except Exception:
        return os.environ.get("GITHUB_SHA", "unknown")


def load_private_key(key_arg: str | None, key_file_arg: str | None, generate_if_missing: Path | None = None) -> ed25519.Ed25519PrivateKey:
    """
    Loads Ed25519 private key from environment, file, or generates ephemeral key.
    Ensures key is never leaked to stdout/stderr.
    """
    raw_key_hex = key_arg or os.environ.get("SPINE_RELEASE_SIGNING_KEY")
    key_file = key_file_arg or os.environ.get("SPINE_RELEASE_SIGNING_KEY_FILE")

    if raw_key_hex:
        raw_key_hex = raw_key_hex.strip()
        try:
            key_bytes = bytes.fromhex(raw_key_hex)
            return ed25519.Ed25519PrivateKey.from_private_bytes(key_bytes)
        except Exception as e:
            sys.stderr.write("ERROR: Invalid private key format provided in argument or environment.\n")
            sys.exit(1)

    if key_file:
        kp = Path(key_file)
        if kp.exists():
            content = kp.read_bytes().strip()
            # Try raw bytes (32 bytes) or hex string
            if len(content) == 32:
                return ed25519.Ed25519PrivateKey.from_private_bytes(content)
            try:
                hex_str = content.decode("utf-8").strip()
                return ed25519.Ed25519PrivateKey.from_private_bytes(bytes.fromhex(hex_str))
            except Exception:
                try:
                    return serialization.load_pem_private_key(content, password=None)
                except Exception:
                    sys.stderr.write("ERROR: Unable to parse private key file.\n")
                    sys.exit(1)
        else:
            sys.stderr.write(f"ERROR: Specified private key file does not exist: {kp}\n")
            sys.exit(1)

    if generate_if_missing:
        # Generate new key and save to specified path
        priv = ed25519.Ed25519PrivateKey.generate()
        priv_bytes = priv.private_bytes(
            encoding=serialization.Encoding.Raw,
            format=serialization.PrivateFormat.Raw,
            encryption_algorithm=serialization.NoEncryption(),
        )
        generate_if_missing.parent.mkdir(parents=True, exist_ok=True)
        # Write with 0600 permissions
        generate_if_missing.write_text(priv_bytes.hex())
        try:
            os.chmod(generate_if_missing, 0o600)
        except Exception:
            pass
        return priv

    sys.stderr.write("ERROR: No release signing key provided. Set SPINE_RELEASE_SIGNING_KEY, SPINE_RELEASE_SIGNING_KEY_FILE, or pass --key-file.\n")
    sys.exit(1)


def sign_release_artifacts(
    artifacts_dir: Path,
    private_key: ed25519.Ed25519PrivateKey,
    public_key_out: Path | None = None,
) -> None:
    manifest_path = artifacts_dir / "SHA256SUMS"
    if not manifest_path.exists():
        sys.stderr.write(f"ERROR: Manifest {manifest_path} not found. Cannot sign release artifacts.\n")
        sys.exit(1)

    pub_key = private_key.public_key()
    pub_bytes = pub_key.public_bytes(
        encoding=serialization.Encoding.Raw,
        format=serialization.PublicFormat.Raw,
    )
    pub_hex = pub_bytes.hex()

    if public_key_out:
        public_key_out.parent.mkdir(parents=True, exist_ok=True)
        public_key_out.write_text(pub_hex + "\n")
        print(f"[SIGN] Public key saved to {public_key_out}")

    # 1. Parse SHA256SUMS and verify on-disk files match checksums
    lines = manifest_path.read_text(encoding="utf-8").splitlines()
    entries = []
    for line in lines:
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split(None, 1)
        if len(parts) == 2:
            expected_hash, filename = parts[0], parts[1].strip("* ")
            entries.append((expected_hash.lower(), filename))

    if not entries:
        sys.stderr.write("ERROR: SHA256SUMS is empty or contains no valid entries.\n")
        sys.exit(1)

    artifact_records = []
    for expected_hash, filename in entries:
        file_path = artifacts_dir / filename
        if not file_path.exists():
            sys.stderr.write(f"ERROR: Referenced release file does not exist: {file_path}\n")
            sys.exit(1)
        actual_hash = compute_sha256(file_path).lower()
        if actual_hash != expected_hash:
            sys.stderr.write(f"ERROR: Hash mismatch for {filename}! Expected {expected_hash}, calculated {actual_hash}\n")
            sys.exit(1)

        # Sign individual binary
        file_data = file_path.read_bytes()
        sig = private_key.sign(file_data)
        sig_path = artifacts_dir / f"{filename}.sig"
        sig_path.write_bytes(sig)
        print(f"[SIGN] Signed {filename} -> {sig_path.name} (Ed25519, {len(sig)} bytes)")

        artifact_records.append({
            "name": filename,
            "sha256": actual_hash,
            "size_bytes": len(file_data),
            "signature_hex": sig.hex(),
            "signature_file": f"{filename}.sig",
        })

    # 2. Sign SHA256SUMS manifest itself
    manifest_bytes = manifest_path.read_bytes()
    manifest_sig = private_key.sign(manifest_bytes)
    manifest_sig_path = artifacts_dir / "SHA256SUMS.sig"
    manifest_sig_path.write_bytes(manifest_sig)
    print(f"[SIGN] Signed SHA256SUMS -> {manifest_sig_path.name} (Ed25519, {len(manifest_sig)} bytes)")

    # 3. Generate release-provenance.json
    commit_sha = get_git_commit(artifacts_dir)
    provenance = {
        "schema_version": "1.0.0",
        "timestamp_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "git_commit": commit_sha,
        "algorithm": "ed25519",
        "public_key_hex": pub_hex,
        "manifest": {
            "file": "SHA256SUMS",
            "sha256": hashlib.sha256(manifest_bytes).hexdigest(),
            "signature_hex": manifest_sig.hex(),
            "signature_file": "SHA256SUMS.sig",
        },
        "artifacts": artifact_records,
    }

    provenance_path = artifacts_dir / "release-provenance.json"
    prov_bytes = (json.dumps(provenance, indent=2) + "\n").encode("utf-8")
    provenance_path.write_bytes(prov_bytes)
    print(f"[SIGN] Created provenance metadata -> {provenance_path.name}")

    # 4. Sign release-provenance.json itself to bind provenance to trusted key
    prov_sig = private_key.sign(prov_bytes)
    prov_sig_path = artifacts_dir / "release-provenance.json.sig"
    prov_sig_path.write_bytes(prov_sig)
    print(f"[SIGN] Signed provenance metadata -> {prov_sig_path.name} (Ed25519, {len(prov_sig)} bytes)")

    print(f"[SUCCESS] All {len(artifact_records)} artifacts, manifest, and provenance signed with Ed25519.")


def main():
    parser = argparse.ArgumentParser(description="Sign Spine release binaries and manifest.")
    parser.add_argument("--dir", type=Path, default=Path("target/release-candidate"), help="Directory containing release artifacts")
    parser.add_argument("--key", type=str, default=None, help="Raw private key hex (recommend using env var instead)")
    parser.add_argument("--key-file", type=str, default=None, help="Path to private key file")
    parser.add_argument("--generate-key-if-missing", type=Path, default=None, help="Generate new private key at path if missing")
    parser.add_argument("--public-key-out", type=Path, default=None, help="Path to output public key hex")

    args = parser.parse_args()

    priv_key = load_private_key(args.key, args.key_file, args.generate_key_if_missing)
    sign_release_artifacts(args.dir, priv_key, args.public_key_out)


if __name__ == "__main__":
    main()
