#!/usr/bin/env python3
"""
Spine Release Artifact Verification Script
Verifies SHA256 checksums, Ed25519 signatures, and provenance metadata of release artifacts.
Fails with exit code 1 if any binary, manifest, or signature has been tampered with or corrupted.
"""

import argparse
import hashlib
import json
import os
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
    h = hashlib.sha256()
    with open(filepath, "rb") as f:
        while chunk := f.read(65536):
            h.update(chunk)
    return h.hexdigest()


def load_public_key(pub_key_str: str | None, pub_key_file: str | None, artifacts_dir: Path) -> ed25519.Ed25519PublicKey:
    raw_pub_hex = pub_key_str or os.environ.get("SPINE_RELEASE_PUBLIC_KEY")
    key_file = pub_key_file or os.environ.get("SPINE_RELEASE_PUBLIC_KEY_FILE")

    if not raw_pub_hex and not key_file:
        candidate_file = artifacts_dir / "SPINE_RELEASE_PUBLIC_KEY.pub"
        if candidate_file.exists():
            key_file = str(candidate_file)

    if raw_pub_hex:
        raw_pub_hex = raw_pub_hex.strip()
        try:
            return ed25519.Ed25519PublicKey.from_public_bytes(bytes.fromhex(raw_pub_hex))
        except Exception:
            sys.stderr.write("ERROR: Invalid public key hex string.\n")
            sys.exit(1)

    if key_file:
        kf = Path(key_file)
        if not kf.exists():
            sys.stderr.write(f"ERROR: Public key file not found: {kf}\n")
            sys.exit(1)
        content = kf.read_bytes().strip()
        if len(content) == 32:
            return ed25519.Ed25519PublicKey.from_public_bytes(content)
        try:
            hex_str = content.decode("utf-8").strip()
            return ed25519.Ed25519PublicKey.from_public_bytes(bytes.fromhex(hex_str))
        except Exception:
            try:
                return serialization.load_pem_public_key(content)
            except Exception:
                sys.stderr.write(f"ERROR: Could not parse public key from file {kf}\n")
                sys.exit(1)

    # Check release-provenance.json as fallback if public key was stored there
    provenance_path = artifacts_dir / "release-provenance.json"
    if provenance_path.exists():
        try:
            data = json.loads(provenance_path.read_text(encoding="utf-8"))
            if "public_key_hex" in data:
                return ed25519.Ed25519PublicKey.from_public_bytes(bytes.fromhex(data["public_key_hex"]))
        except Exception:
            pass

    sys.stderr.write("ERROR: No public key provided for signature verification. Provide --public-key, --public-key-file, or set SPINE_RELEASE_PUBLIC_KEY.\n")
    sys.exit(1)


def verify_signature(pub_key: ed25519.Ed25519PublicKey, signature_bytes: bytes, data_bytes: bytes) -> bool:
    try:
        pub_key.verify(signature_bytes, data_bytes)
        return True
    except InvalidSignature:
        return False
    except Exception:
        return False


def verify_release(artifacts_dir: Path, pub_key: ed25519.Ed25519PublicKey) -> bool:
    manifest_path = artifacts_dir / "SHA256SUMS"
    manifest_sig_path = artifacts_dir / "SHA256SUMS.sig"

    if not manifest_path.exists():
        sys.stderr.write(f"[FAIL] Missing manifest file: {manifest_path}\n")
        return False

    if not manifest_sig_path.exists():
        sys.stderr.write(f"[FAIL] Missing manifest signature: {manifest_sig_path}\n")
        return False

    manifest_bytes = manifest_path.read_bytes()
    manifest_sig_bytes = manifest_sig_path.read_bytes()

    if not verify_signature(pub_key, manifest_sig_bytes, manifest_bytes):
        sys.stderr.write("[FAIL] Cryptographic verification failed for SHA256SUMS.sig! Manifest has been tampered with or signed with a different key.\n")
        return False

    print(f"[PASS] Manifest signature verified: {manifest_sig_path.name}")

    # Parse manifest entries
    lines = manifest_path.read_text(encoding="utf-8").splitlines()
    entries = []
    for line in lines:
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split(None, 1)
        if len(parts) == 2:
            expected_hash, filename = parts[0].lower(), parts[1].strip("* ")
            entries.append((expected_hash, filename))

    if not entries:
        sys.stderr.write("[FAIL] No valid entries found in SHA256SUMS.\n")
        return False

    failures = 0
    for expected_hash, filename in entries:
        file_path = artifacts_dir / filename
        sig_path = artifacts_dir / f"{filename}.sig"

        if not file_path.exists():
            sys.stderr.write(f"[FAIL] Artifact does not exist: {file_path}\n")
            failures += 1
            continue

        actual_hash = compute_sha256(file_path).lower()
        if actual_hash != expected_hash:
            sys.stderr.write(f"[FAIL] Hash mismatch for {filename}: expected {expected_hash}, calculated {actual_hash}\n")
            failures += 1
            continue

        if not sig_path.exists():
            sys.stderr.write(f"[FAIL] Missing signature file for {filename}: {sig_path}\n")
            failures += 1
            continue

        sig_bytes = sig_path.read_bytes()
        file_bytes = file_path.read_bytes()
        if not verify_signature(pub_key, sig_bytes, file_bytes):
            sys.stderr.write(f"[FAIL] Cryptographic signature mismatch for artifact: {filename}! Binary has been modified or signature is invalid.\n")
            failures += 1
            continue

        print(f"[PASS] Verified checksum and Ed25519 signature: {filename} ({len(file_bytes)} bytes)")

    # Verify provenance JSON if present
    provenance_path = artifacts_dir / "release-provenance.json"
    if provenance_path.exists():
        try:
            prov = json.loads(provenance_path.read_text(encoding="utf-8"))
            prov_manifest = prov.get("manifest", {})
            if prov_manifest.get("sha256") != hashlib.sha256(manifest_bytes).hexdigest():
                sys.stderr.write("[FAIL] Provenance manifest sha256 does not match SHA256SUMS file.\n")
                failures += 1
            else:
                print("[PASS] Verified release-provenance.json metadata coherence.")
        except Exception as e:
            sys.stderr.write(f"[FAIL] Unable to parse provenance JSON: {e}\n")
            failures += 1

    if failures > 0:
        sys.stderr.write(f"\n[SUMMARY] Verification failed with {failures} error(s).\n")
        return False

    print(f"\n[SUMMARY] All {len(entries)} artifacts, SHA256SUMS manifest, and cryptographic signatures successfully verified.")
    return True


def main():
    parser = argparse.ArgumentParser(description="Verify Spine release binaries and signatures.")
    parser.add_argument("--dir", type=Path, default=Path("target/release-candidate"), help="Directory containing release artifacts")
    parser.add_argument("--public-key", type=str, default=None, help="Raw public key hex")
    parser.add_argument("--public-key-file", type=str, default=None, help="Path to public key file")

    args = parser.parse_args()

    pub_key = load_public_key(args.public_key, args.public_key_file, args.dir)
    success = verify_release(args.dir, pub_key)
    if not success:
        sys.exit(1)
    sys.exit(0)


if __name__ == "__main__":
    main()
