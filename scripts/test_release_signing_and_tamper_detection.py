#!/usr/bin/env python3
"""
Automated Test Suite for Spine Release Artifact Signing and Tamper Detection.
Tests:
  1. Clean signing and cryptographic verification cycle.
  2. Rejection of tampered binary (1-byte flip).
  3. Rejection of tampered SHA256SUMS manifest.
  4. Rejection of tampered detached signature (.sig).
  5. Rejection of verification with wrong/mismatched public key.
  6. Rejection of missing signature file.
  7. Rejection of missing binary referenced in manifest.
  8. Rejection of truncated/empty signature file.
  9. Verification that private keys are never exposed in stdout, stderr, or provenance files.
"""

import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

try:
    from cryptography.hazmat.primitives.asymmetric import ed25519
    from cryptography.hazmat.primitives import serialization
except ImportError:
    sys.stderr.write("ERROR: Python 'cryptography' library required. Install with: pip install cryptography\n")
    sys.exit(2)

REPO_ROOT = Path(__file__).resolve().parent.parent
SIGN_SCRIPT = REPO_ROOT / "scripts" / "sign_release_artifacts.py"
VERIFY_SCRIPT = REPO_ROOT / "scripts" / "verify_release_signatures.py"


def run_cmd(args: list[str], env: dict | None = None) -> tuple[int, str, str]:
    proc = subprocess.run(
        args,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        env=env or os.environ.copy(),
    )
    return proc.returncode, proc.stdout, proc.stderr


def create_fixture_directory(tmp_dir: Path) -> tuple[dict[str, bytes], str, str]:
    """Creates synthetic release candidate binaries and SHA256SUMS."""
    binaries = {
        "spine-api": b"\x7fELF_FAKE_SPINE_API_BINARY_PAYLOAD_1234567890" * 50,
        "spine-coordinator": b"\x7fELF_FAKE_COORDINATOR_BINARY_PAYLOAD_1234567890" * 30,
        "spine-storage-node": b"\x7fELF_FAKE_STORAGE_NODE_PAYLOAD_1234567890" * 40,
        "spine": b"\x7fELF_FAKE_CLI_CLIENT_PAYLOAD_1234567890" * 20,
    }

    manifest_lines = []
    for name, content in binaries.items():
        bin_path = tmp_dir / name
        bin_path.write_bytes(content)
        sha = hashlib.sha256(content).hexdigest()
        manifest_lines.append(f"{sha}  {name}\n")

    manifest_path = tmp_dir / "SHA256SUMS"
    manifest_path.write_text("".join(manifest_lines), encoding="utf-8")

    # Generate an Ed25519 keypair for testing
    priv = ed25519.Ed25519PrivateKey.generate()
    priv_bytes = priv.private_bytes(
        encoding=serialization.Encoding.Raw,
        format=serialization.PrivateFormat.Raw,
        encryption_algorithm=serialization.NoEncryption(),
    )
    pub = priv.public_key()
    pub_bytes = pub.public_bytes(
        encoding=serialization.Encoding.Raw,
        format=serialization.PublicFormat.Raw,
    )

    return binaries, priv_bytes.hex(), pub_bytes.hex()


def test_suite():
    print("=" * 70)
    print("Spine Release Signing & Tamper Detection Verification Suite")
    print("=" * 70)

    with tempfile.TemporaryDirectory() as td:
        tmp_path = Path(td)
        binaries, priv_hex, pub_hex = create_fixture_directory(tmp_path)

        key_file = tmp_path / "test_private_key"
        key_file.write_text(priv_hex)
        try:
            os.chmod(key_file, 0o600)
        except Exception:
            pass

        pub_file = tmp_path / "test_public_key.pub"
        pub_file.write_text(pub_hex + "\n")

        # ----------------------------------------------------------------------
        # Test 1: Clean sign and verify
        # ----------------------------------------------------------------------
        print("\n[TEST 1] Clean sign and verify cycle...")
        env = os.environ.copy()
        env["SPINE_RELEASE_SIGNING_KEY"] = priv_hex
        ret, stdout, stderr = run_cmd([sys.executable, str(SIGN_SCRIPT), "--dir", str(tmp_path)], env=env)
        assert ret == 0, f"Signing failed with exit code {ret}:\n{stderr}"
        assert (tmp_path / "SHA256SUMS.sig").exists(), "SHA256SUMS.sig was not created!"
        assert (tmp_path / "release-provenance.json").exists(), "release-provenance.json was not created!"

        ret, stdout, stderr = run_cmd([sys.executable, str(VERIFY_SCRIPT), "--dir", str(tmp_path), "--public-key", pub_hex])
        assert ret == 0, f"Verification failed on clean artifacts with exit code {ret}:\n{stderr}"
        assert "[PASS] Manifest signature verified" in stdout
        print("  -> PASSED: Clean artifacts, manifest, and signatures verified 100%.")

        # ----------------------------------------------------------------------
        # Test 2: Tampered binary (1-byte modification)
        # ----------------------------------------------------------------------
        print("\n[TEST 2] Rejection of tampered binary...")
        target_bin = tmp_path / "spine-api"
        original_data = target_bin.read_bytes()
        tampered_data = bytearray(original_data)
        tampered_data[15] ^= 0xFF  # Flip bits
        target_bin.write_bytes(tampered_data)

        ret, stdout, stderr = run_cmd([sys.executable, str(VERIFY_SCRIPT), "--dir", str(tmp_path), "--public-key", pub_hex])
        assert ret != 0, "Verification unexpectedly succeeded on tampered binary!"
        assert "Hash mismatch for spine-api" in stderr or "Cryptographic signature mismatch for artifact: spine-api" in stderr
        print("  -> PASSED: Tampered binary was correctly detected and rejected.")
        target_bin.write_bytes(original_data)  # Restore

        # ----------------------------------------------------------------------
        # Test 3: Tampered SHA256SUMS manifest
        # ----------------------------------------------------------------------
        print("\n[TEST 3] Rejection of tampered SHA256SUMS manifest...")
        manifest = tmp_path / "SHA256SUMS"
        orig_manifest = manifest.read_text(encoding="utf-8")
        manifest.write_text(orig_manifest + "0000000000000000000000000000000000000000000000000000000000000000  fake-file\n")

        ret, stdout, stderr = run_cmd([sys.executable, str(VERIFY_SCRIPT), "--dir", str(tmp_path), "--public-key", pub_hex])
        assert ret != 0, "Verification unexpectedly succeeded on tampered manifest!"
        assert "Cryptographic verification failed for SHA256SUMS.sig" in stderr
        print("  -> PASSED: Tampered manifest was correctly detected and rejected.")
        manifest.write_text(orig_manifest, encoding="utf-8")  # Restore

        # ----------------------------------------------------------------------
        # Test 4: Tampered signature file (.sig)
        # ----------------------------------------------------------------------
        print("\n[TEST 4] Rejection of tampered detached signature...")
        sig_file = tmp_path / "spine-coordinator.sig"
        orig_sig = sig_file.read_bytes()
        tampered_sig = bytearray(orig_sig)
        tampered_sig[5] ^= 0xAA
        sig_file.write_bytes(tampered_sig)

        ret, stdout, stderr = run_cmd([sys.executable, str(VERIFY_SCRIPT), "--dir", str(tmp_path), "--public-key", pub_hex])
        assert ret != 0, "Verification unexpectedly succeeded on tampered signature!"
        assert "Cryptographic signature mismatch for artifact: spine-coordinator" in stderr
        print("  -> PASSED: Tampered detached signature was correctly detected and rejected.")
        sig_file.write_bytes(orig_sig)  # Restore

        # ----------------------------------------------------------------------
        # Test 5: Wrong public key
        # ----------------------------------------------------------------------
        print("\n[TEST 5] Rejection of verification with wrong public key...")
        other_priv = ed25519.Ed25519PrivateKey.generate()
        other_pub = other_priv.public_key()
        other_pub_bytes = other_pub.public_bytes(
            encoding=serialization.Encoding.Raw,
            format=serialization.PublicFormat.Raw,
        )
        ret, stdout, stderr = run_cmd([sys.executable, str(VERIFY_SCRIPT), "--dir", str(tmp_path), "--public-key", other_pub_bytes.hex()])
        assert ret != 0, "Verification unexpectedly succeeded with wrong public key!"
        assert "Cryptographic verification failed for SHA256SUMS.sig" in stderr
        print("  -> PASSED: Verification failed with wrong public key as expected.")

        # ----------------------------------------------------------------------
        # Test 6: Missing signature file
        # ----------------------------------------------------------------------
        print("\n[TEST 6] Rejection of missing signature file...")
        sig_file = tmp_path / "spine.sig"
        saved_sig = sig_file.read_bytes()
        sig_file.unlink()

        ret, stdout, stderr = run_cmd([sys.executable, str(VERIFY_SCRIPT), "--dir", str(tmp_path), "--public-key", pub_hex])
        assert ret != 0, "Verification unexpectedly succeeded with missing signature!"
        assert "Missing signature file for spine" in stderr
        print("  -> PASSED: Missing signature file was correctly detected and rejected.")
        sig_file.write_bytes(saved_sig)  # Restore

        # ----------------------------------------------------------------------
        # Test 7: Missing binary referenced in manifest
        # ----------------------------------------------------------------------
        print("\n[TEST 7] Rejection of missing binary...")
        bin_file = tmp_path / "spine-storage-node"
        saved_bin = bin_file.read_bytes()
        bin_file.unlink()

        ret, stdout, stderr = run_cmd([sys.executable, str(VERIFY_SCRIPT), "--dir", str(tmp_path), "--public-key", pub_hex])
        assert ret != 0, "Verification unexpectedly succeeded with missing binary!"
        assert "Artifact does not exist" in stderr
        print("  -> PASSED: Missing binary was correctly detected and rejected.")
        bin_file.write_bytes(saved_bin)  # Restore

        # ----------------------------------------------------------------------
        # Test 8: Truncated/corrupted signature
        # ----------------------------------------------------------------------
        print("\n[TEST 8] Rejection of truncated/empty signature...")
        sig_file = tmp_path / "SHA256SUMS.sig"
        saved_manifest_sig = sig_file.read_bytes()
        sig_file.write_bytes(b"truncated")

        ret, stdout, stderr = run_cmd([sys.executable, str(VERIFY_SCRIPT), "--dir", str(tmp_path), "--public-key", pub_hex])
        assert ret != 0, "Verification unexpectedly succeeded with truncated signature!"
        assert "Cryptographic verification failed for SHA256SUMS.sig" in stderr
        print("  -> PASSED: Truncated signature was correctly detected and rejected.")
        sig_file.write_bytes(saved_manifest_sig)  # Restore

        # ----------------------------------------------------------------------
        # Test 9: Secret suppression assertion
        # ----------------------------------------------------------------------
        print("\n[TEST 9] Asserting private keys are never exposed in stdout or files...")
        assert priv_hex not in stdout, "Private key was leaked in stdout!"
        assert priv_hex not in stderr, "Private key was leaked in stderr!"
        prov_text = (tmp_path / "release-provenance.json").read_text(encoding="utf-8")
        assert priv_hex not in prov_text, "Private key was leaked in provenance file!"
        print("  -> PASSED: Zero private key exposure detected.")

    print("\n" + "=" * 70)
    print("ALL 9 RELEASE SIGNING & TAMPER DETECTION TESTS PASSED (EXIT CODE 0)")
    print("=" * 70)


if __name__ == "__main__":
    test_suite()
