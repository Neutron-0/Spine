#!/usr/bin/env python3
"""
Spine Release Artifact Verification Script (Independent Trust Anchor)
Verifies SHA256 checksums, Ed25519 signatures, and provenance metadata of release artifacts.

Trust Architecture:
  Verification strictly requires an independently authenticated trust anchor (public key).
  It NEVER blindly trusts an unauthenticated public key bundled inside an untrusted artifact directory.
  If a bundled key is present, it MUST match the independent trust anchor bit-for-bit.
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

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_TRUST_ANCHOR = REPO_ROOT / "deploy" / "security" / "spine-release-signing.pub"


def compute_sha256(filepath: Path) -> str:
    h = hashlib.sha256()
    with open(filepath, "rb") as f:
        while chunk := f.read(65536):
            h.update(chunk)
    return h.hexdigest()


def parse_key_bytes(raw_content: bytes | str) -> bytes:
    """Parses raw 32-byte key or hex string (ignoring comments/whitespace)."""
    if isinstance(raw_content, str):
        lines = [line.strip() for line in raw_content.splitlines() if line.strip() and not line.startswith("#")]
        hex_candidate = "".join(lines)
        return bytes.fromhex(hex_candidate)
    if len(raw_content) == 32:
        return raw_content
    # Try string decode
    try:
        text = raw_content.decode("utf-8")
        lines = [line.strip() for line in text.splitlines() if line.strip() and not line.startswith("#")]
        hex_candidate = "".join(lines)
        return bytes.fromhex(hex_candidate)
    except Exception:
        raise ValueError("Invalid public key format: expected 32 raw bytes or 64-char hex string.")


def resolve_trust_anchor(
    trusted_key_arg: str | None,
    trusted_key_file_arg: str | None,
    artifacts_dir: Path,
    allow_insecure_bundled: bool = False,
) -> tuple[ed25519.Ed25519PublicKey, str, str]:
    """
    Resolves the independently authenticated trust anchor.
    Returns (public_key_obj, key_hex, trust_source_description).
    """
    raw_key = trusted_key_arg or os.environ.get("SPINE_TRUSTED_PUBLIC_KEY")
    key_file = trusted_key_file_arg or os.environ.get("SPINE_TRUSTED_PUBLIC_KEY_FILE")

    if raw_key:
        raw_key = raw_key.strip()
        try:
            key_bytes = parse_key_bytes(raw_key)
            pub = ed25519.Ed25519PublicKey.from_public_bytes(key_bytes)
            return pub, key_bytes.hex(), "Explicit CLI argument / SPINE_TRUSTED_PUBLIC_KEY environment"
        except Exception as e:
            sys.stderr.write(f"ERROR: Invalid trusted public key string: {e}\n")
            sys.exit(1)

    if key_file:
        kf = Path(key_file)
        if not kf.exists():
            sys.stderr.write(f"ERROR: Specified trusted key file does not exist: {kf}\n")
            sys.exit(1)
        try:
            key_bytes = parse_key_bytes(kf.read_bytes())
            pub = ed25519.Ed25519PublicKey.from_public_bytes(key_bytes)
            return pub, key_bytes.hex(), f"Independently specified file ({kf})"
        except Exception as e:
            sys.stderr.write(f"ERROR: Failed parsing trusted key file {kf}: {e}\n")
            sys.exit(1)

    # Check repository default trust anchor
    if DEFAULT_TRUST_ANCHOR.exists():
        try:
            key_bytes = parse_key_bytes(DEFAULT_TRUST_ANCHOR.read_bytes())
            pub = ed25519.Ed25519PublicKey.from_public_bytes(key_bytes)
            return pub, key_bytes.hex(), f"Repository pinned trust anchor ({DEFAULT_TRUST_ANCHOR.relative_to(REPO_ROOT)})"
        except Exception as e:
            sys.stderr.write(f"ERROR: Failed parsing repository trust anchor {DEFAULT_TRUST_ANCHOR}: {e}\n")
            sys.exit(1)

    # Insecure override check
    if allow_insecure_bundled:
        bundled_file = artifacts_dir / "SPINE_RELEASE_PUBLIC_KEY.pub"
        if bundled_file.exists():
            try:
                key_bytes = parse_key_bytes(bundled_file.read_bytes())
                pub = ed25519.Ed25519PublicKey.from_public_bytes(key_bytes)
                sys.stderr.write("[SECURITY WARNING] Using bundled public key because --insecure-allow-untrusted-bundled-key was passed.\n")
                return pub, key_bytes.hex(), "INSECURE BUNDLED KEY (UNTRUSTED)"
            except Exception as e:
                sys.stderr.write(f"ERROR: Failed parsing bundled key: {e}\n")
                sys.exit(1)

    sys.stderr.write("\n" + "=" * 70 + "\n")
    sys.stderr.write("CRITICAL SECURITY ERROR: Independent Trust Anchor Missing!\n")
    sys.stderr.write("=" * 70 + "\n")
    sys.stderr.write("Verification requires an independently authenticated public key.\n")
    sys.stderr.write("An unauthenticated public key bundled inside an untrusted distribution directory\n")
    sys.stderr.write("cannot establish trust (an attacker who tampered with binaries could simply\n")
    sys.stderr.write("replace the bundled public key).\n\n")
    sys.stderr.write("To verify safely, provide the trust anchor:\n")
    sys.stderr.write("  --trusted-key-file /path/to/pinned/spine-release-signing.pub\n")
    sys.stderr.write("  or set SPINE_TRUSTED_PUBLIC_KEY=<hex>\n")
    sys.stderr.write("=" * 70 + "\n\n")
    sys.exit(1)


def verify_signature(pub_key: ed25519.Ed25519PublicKey, signature_bytes: bytes, data_bytes: bytes) -> bool:
    try:
        pub_key.verify(signature_bytes, data_bytes)
        return True
    except InvalidSignature:
        return False
    except Exception:
        return False


def verify_release(
    artifacts_dir: Path,
    pub_key: ed25519.Ed25519PublicKey,
    trusted_hex: str,
    trust_source: str,
) -> bool:
    print(f"[TRUST ANCHOR] Source: {trust_source}")
    print(f"[TRUST ANCHOR] Public Key (Ed25519): {trusted_hex}")

    # 1. Audit bundled key if present
    bundled_file = artifacts_dir / "SPINE_RELEASE_PUBLIC_KEY.pub"
    if bundled_file.exists():
        try:
            bundled_bytes = parse_key_bytes(bundled_file.read_bytes())
            bundled_hex = bundled_bytes.hex()
            if bundled_hex.lower() != trusted_hex.lower():
                sys.stderr.write("\n" + "!" * 70 + "\n")
                sys.stderr.write("[CRITICAL TAMPER ALERT] Bundled public key does NOT match trusted anchor!\n")
                sys.stderr.write(f"  Bundled Key: {bundled_hex}\n")
                sys.stderr.write(f"  Trusted Key: {trusted_hex}\n")
                sys.stderr.write("This indicates the release bundle was generated or modified with an unauthorized key!\n")
                sys.stderr.write("!" * 70 + "\n\n")
                return False
            print("[PASS] Bundled public key matches the independent trust anchor bit-for-bit.")
        except Exception as e:
            sys.stderr.write(f"[FAIL] Unable to parse bundled public key: {e}\n")
            return False

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
        sys.stderr.write("[FAIL] Cryptographic verification failed for SHA256SUMS.sig! Manifest was NOT signed by trusted anchor.\n")
        return False

    print(f"[PASS] Manifest signature verified with trusted anchor: {manifest_sig_path.name}")

    # 2. Parse manifest entries
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
            sys.stderr.write(f"[FAIL] Missing detached signature for {filename}: {sig_path}\n")
            failures += 1
            continue

        sig_bytes = sig_path.read_bytes()
        file_bytes = file_path.read_bytes()
        if not verify_signature(pub_key, sig_bytes, file_bytes):
            sys.stderr.write(f"[FAIL] Cryptographic signature mismatch for {filename}! File was NOT signed by trusted anchor.\n")
            failures += 1
            continue

        print(f"[PASS] Verified checksum and Ed25519 signature: {filename} ({len(file_bytes)} bytes)")

    # 3. Authenticate Provenance Metadata & Detached Provenance Signature
    provenance_path = artifacts_dir / "release-provenance.json"
    prov_sig_path = artifacts_dir / "release-provenance.json.sig"

    if provenance_path.exists():
        prov_bytes = provenance_path.read_bytes()
        if prov_sig_path.exists():
            prov_sig_bytes = prov_sig_path.read_bytes()
            if not verify_signature(pub_key, prov_sig_bytes, prov_bytes):
                sys.stderr.write("[FAIL] Cryptographic verification failed for release-provenance.json.sig! Provenance has been tampered with.\n")
                failures += 1
            else:
                print(f"[PASS] Provenance metadata signature verified: {prov_sig_path.name}")
        else:
            sys.stderr.write(f"[FAIL] Missing detached signature for provenance metadata: {prov_sig_path}\n")
            failures += 1

        try:
            prov = json.loads(prov_bytes.decode("utf-8"))
            prov_key = prov.get("public_key_hex", "").strip().lower()
            if prov_key != trusted_hex.lower():
                sys.stderr.write(f"[FAIL] Provenance public_key_hex ({prov_key}) does NOT match trusted anchor ({trusted_hex})!\n")
                failures += 1

            prov_manifest_hash = prov.get("manifest", {}).get("sha256", "").strip().lower()
            actual_manifest_hash = hashlib.sha256(manifest_bytes).hexdigest().lower()
            if prov_manifest_hash != actual_manifest_hash:
                sys.stderr.write(f"[FAIL] Provenance manifest hash ({prov_manifest_hash}) does not match SHA256SUMS ({actual_manifest_hash})!\n")
                failures += 1

            # Validate each artifact in provenance
            prov_artifacts = {a.get("name"): a.get("sha256", "").lower() for a in prov.get("artifacts", [])}
            for exp_hash, fname in entries:
                if fname not in prov_artifacts:
                    sys.stderr.write(f"[FAIL] Artifact {fname} is missing from release-provenance.json!\n")
                    failures += 1
                elif prov_artifacts[fname] != exp_hash:
                    sys.stderr.write(f"[FAIL] Hash mismatch for {fname} between manifest ({exp_hash}) and provenance ({prov_artifacts[fname]})!\n")
                    failures += 1

            print(f"[PASS] Verified release-provenance.json bound to commit {prov.get('git_commit', 'unknown')} and all artifact digests.")
        except Exception as e:
            sys.stderr.write(f"[FAIL] Unable to parse provenance JSON: {e}\n")
            failures += 1

    if failures > 0:
        sys.stderr.write(f"\n[SUMMARY] Verification failed with {failures} error(s).\n")
        return False

    print(f"\n[SUMMARY] All {len(entries)} artifacts, SHA256SUMS manifest, and provenance authenticated against independent trust anchor.")
    return True


def main():
    parser = argparse.ArgumentParser(description="Verify Spine release binaries and signatures against an independent trust anchor.")
    parser.add_argument("--dir", type=Path, default=Path("target/release-candidate"), help="Directory containing release artifacts")
    parser.add_argument("--trusted-key", type=str, default=None, help="Authoritative public key hex")
    parser.add_argument("--trusted-key-file", type=str, default=None, help="Path to authoritative public key file")
    parser.add_argument("--insecure-allow-untrusted-bundled-key", action="store_true", help="INSECURE: Trust public key found inside artifact directory (testing only)")

    args = parser.parse_args()

    pub_key, key_hex, trust_source = resolve_trust_anchor(
        args.trusted_key,
        args.trusted_key_file,
        args.dir,
        args.insecure_allow_untrusted_bundled_key,
    )
    success = verify_release(args.dir, pub_key, key_hex, trust_source)
    if not success:
        sys.exit(1)
    sys.exit(0)


if __name__ == "__main__":
    main()
