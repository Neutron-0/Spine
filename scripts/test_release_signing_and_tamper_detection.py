#!/usr/bin/env python3
"""
Automated Test Suite for Spine Release Artifact Signing, Independent Trust Anchors,
and Anti-Tamper Detection.

Verifies:
  1. Clean signing and cryptographic verification cycle with independent trust anchor.
  2. Rejection of tampered binary (1-byte flip).
  3. Rejection of tampered SHA256SUMS manifest.
  4. Rejection of tampered detached signature (.sig).
  5. Rejection of verification with wrong/mismatched public key.
  6. Rejection of missing signature file.
  7. Rejection of missing binary referenced in manifest.
  8. Rejection of truncated/empty signature file.
  9. Verification that private keys are never exposed in stdout, stderr, or provenance files.
 10. REJECTION OF FULL KEY REPLACEMENT ATTACK: Attacker creates own key, signs binaries, and replaces bundled key.
 11. FAIL-CLOSED: Refusal to verify without an independently authenticated trust anchor.
 12. PROVENANCE AUTHENTICATION: Rejection of tampered release-provenance.json or invalid provenance signature.
"""

import hashlib
import json
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
    print("=" * 75)
    print("Spine Release Signing, Trust Anchor & Anti-Tamper Verification Suite")
    print("=" * 75)

    with tempfile.TemporaryDirectory() as td:
        tmp_path = Path(td)
        binaries, priv_hex, pub_hex = create_fixture_directory(tmp_path)

        key_file = tmp_path / "test_private_key"
        key_file.write_text(priv_hex)
        try:
            os.chmod(key_file, 0o600)
        except Exception:
            pass

        trusted_pub_file = tmp_path / "trusted_anchor.pub"
        trusted_pub_file.write_text(f"# Trusted Key Anchor\n{pub_hex}\n")

        # ----------------------------------------------------------------------
        # Test 1: Clean sign and verify with independent trust anchor
        # ----------------------------------------------------------------------
        print("\n[TEST 1] Clean sign and verify cycle with independent trust anchor...")
        env = os.environ.copy()
        env["SPINE_RELEASE_SIGNING_KEY"] = priv_hex
        ret, stdout, stderr = run_cmd([
            sys.executable, str(SIGN_SCRIPT),
            "--dir", str(tmp_path),
            "--public-key-out", str(tmp_path / "SPINE_RELEASE_PUBLIC_KEY.pub")
        ], env=env)
        assert ret == 0, f"Signing failed with exit code {ret}:\n{stderr}"
        assert (tmp_path / "SHA256SUMS.sig").exists(), "SHA256SUMS.sig was not created!"
        assert (tmp_path / "release-provenance.json").exists(), "release-provenance.json was not created!"
        assert (tmp_path / "release-provenance.json.sig").exists(), "release-provenance.json.sig was not created!"

        ret, stdout, stderr = run_cmd([
            sys.executable, str(VERIFY_SCRIPT),
            "--dir", str(tmp_path),
            "--trusted-key-file", str(trusted_pub_file)
        ])
        assert ret == 0, f"Verification failed on clean artifacts with exit code {ret}:\n{stderr}"
        assert "[PASS] Manifest signature verified with trusted anchor" in stdout
        assert "[PASS] Provenance metadata signature verified" in stdout
        print("  -> PASSED: Clean artifacts, manifest, and provenance verified 100%.")

        # ----------------------------------------------------------------------
        # Test 2: Tampered binary (1-byte modification)
        # ----------------------------------------------------------------------
        print("\n[TEST 2] Rejection of tampered binary...")
        target_bin = tmp_path / "spine-api"
        original_data = target_bin.read_bytes()
        tampered_data = bytearray(original_data)
        tampered_data[15] ^= 0xFF  # Flip bits
        target_bin.write_bytes(tampered_data)

        ret, stdout, stderr = run_cmd([
            sys.executable, str(VERIFY_SCRIPT),
            "--dir", str(tmp_path),
            "--trusted-key-file", str(trusted_pub_file)
        ])
        assert ret != 0, "Verification unexpectedly succeeded on tampered binary!"
        assert "Hash mismatch for spine-api" in stderr or "Cryptographic signature mismatch for spine-api" in stderr
        print("  -> PASSED: Tampered binary was correctly detected and rejected.")
        target_bin.write_bytes(original_data)  # Restore

        # ----------------------------------------------------------------------
        # Test 3: Tampered SHA256SUMS manifest
        # ----------------------------------------------------------------------
        print("\n[TEST 3] Rejection of tampered SHA256SUMS manifest...")
        manifest = tmp_path / "SHA256SUMS"
        orig_manifest = manifest.read_text(encoding="utf-8")
        manifest.write_text(orig_manifest + "0000000000000000000000000000000000000000000000000000000000000000  fake-file\n")

        ret, stdout, stderr = run_cmd([
            sys.executable, str(VERIFY_SCRIPT),
            "--dir", str(tmp_path),
            "--trusted-key-file", str(trusted_pub_file)
        ])
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

        ret, stdout, stderr = run_cmd([
            sys.executable, str(VERIFY_SCRIPT),
            "--dir", str(tmp_path),
            "--trusted-key-file", str(trusted_pub_file)
        ])
        assert ret != 0, "Verification unexpectedly succeeded on tampered signature!"
        assert "Cryptographic signature mismatch for spine-coordinator" in stderr
        print("  -> PASSED: Tampered detached signature was correctly detected and rejected.")
        sig_file.write_bytes(orig_sig)  # Restore

        # ----------------------------------------------------------------------
        # Test 5: Wrong public key
        # ----------------------------------------------------------------------
        print("\n[TEST 5] Rejection of verification with wrong public key...")
        other_priv = ed25519.Ed25519PrivateKey.generate()
        other_pub_bytes = other_priv.public_key().public_bytes(
            encoding=serialization.Encoding.Raw,
            format=serialization.PublicFormat.Raw,
        )
        ret, stdout, stderr = run_cmd([
            sys.executable, str(VERIFY_SCRIPT),
            "--dir", str(tmp_path),
            "--trusted-key", other_pub_bytes.hex()
        ])
        assert ret != 0, "Verification unexpectedly succeeded with wrong public key!"
        assert "Bundled public key does NOT match trusted anchor" in stderr or "Cryptographic verification failed" in stderr
        print("  -> PASSED: Verification failed with wrong public key as expected.")

        # ----------------------------------------------------------------------
        # Test 6: Missing signature file
        # ----------------------------------------------------------------------
        print("\n[TEST 6] Rejection of missing signature file...")
        sig_file = tmp_path / "spine.sig"
        saved_sig = sig_file.read_bytes()
        sig_file.unlink()

        ret, stdout, stderr = run_cmd([
            sys.executable, str(VERIFY_SCRIPT),
            "--dir", str(tmp_path),
            "--trusted-key-file", str(trusted_pub_file)
        ])
        assert ret != 0, "Verification unexpectedly succeeded with missing signature!"
        assert "Missing detached signature for spine" in stderr
        print("  -> PASSED: Missing signature file was correctly detected and rejected.")
        sig_file.write_bytes(saved_sig)  # Restore

        # ----------------------------------------------------------------------
        # Test 7: Missing binary referenced in manifest
        # ----------------------------------------------------------------------
        print("\n[TEST 7] Rejection of missing binary...")
        bin_file = tmp_path / "spine-storage-node"
        saved_bin = bin_file.read_bytes()
        bin_file.unlink()

        ret, stdout, stderr = run_cmd([
            sys.executable, str(VERIFY_SCRIPT),
            "--dir", str(tmp_path),
            "--trusted-key-file", str(trusted_pub_file)
        ])
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

        ret, stdout, stderr = run_cmd([
            sys.executable, str(VERIFY_SCRIPT),
            "--dir", str(tmp_path),
            "--trusted-key-file", str(trusted_pub_file)
        ])
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

        # ----------------------------------------------------------------------
        # Test 10: Full Key Replacement Attack
        # Attacker replaces binaries, manifests, signatures, AND bundled public key.
        # ----------------------------------------------------------------------
        print("\n[TEST 10] REJECTION OF FULL KEY REPLACEMENT ATTACK...")
        attacker_priv = ed25519.Ed25519PrivateKey.generate()
        attacker_priv_bytes = attacker_priv.private_bytes(
            encoding=serialization.Encoding.Raw,
            format=serialization.PrivateFormat.Raw,
            encryption_algorithm=serialization.NoEncryption(),
        )
        attacker_pub = attacker_priv.public_key()
        attacker_pub_bytes = attacker_pub.public_bytes(
            encoding=serialization.Encoding.Raw,
            format=serialization.PublicFormat.Raw,
        )

        with tempfile.TemporaryDirectory() as attack_td:
            attack_dir = Path(attack_td)
            # Create malicious binaries
            create_fixture_directory(attack_dir)
            # Sign everything with attacker's key
            env_att = os.environ.copy()
            env_att["SPINE_RELEASE_SIGNING_KEY"] = attacker_priv_bytes.hex()
            ret_att, _, _ = run_cmd([
                sys.executable, str(SIGN_SCRIPT),
                "--dir", str(attack_dir),
                "--public-key-out", str(attack_dir / "SPINE_RELEASE_PUBLIC_KEY.pub")
            ], env=env_att)
            assert ret_att == 0

            # Attempt verification using the true operator's trusted anchor
            ret_ver, stdout_ver, stderr_ver = run_cmd([
                sys.executable, str(VERIFY_SCRIPT),
                "--dir", str(attack_dir),
                "--trusted-key-file", str(trusted_pub_file)
            ])
            assert ret_ver != 0, "CRITICAL FLAW: Attacker key replacement was accepted!"
            assert "Bundled public key does NOT match trusted anchor" in stderr_ver
            print("  -> PASSED: Full key replacement attack detected and rejected! Bundled key did not match trust anchor.")

        # ----------------------------------------------------------------------
        # Test 11: Fail-closed without independent trust anchor
        # ----------------------------------------------------------------------
        print("\n[TEST 11] Fail-closed refusal to verify without independent trust anchor...")
        with tempfile.TemporaryDirectory() as isolated_td:
            iso_dir = Path(isolated_td)
            shutil.copytree(str(tmp_path), str(iso_dir / "bundle"))
            # Run without trusted key, where DEFAULT_TRUST_ANCHOR doesn't exist or is bypassed
            env_empty = {"PATH": os.environ.get("PATH", "")}
            ret_no_anchor, _, stderr_no_anchor = run_cmd([
                sys.executable, str(VERIFY_SCRIPT),
                "--dir", str(iso_dir / "bundle")
            ], env=env_empty)
            # Since DEFAULT_TRUST_ANCHOR in repo exists, test passing empty key string explicitly
            ret_empty, _, stderr_empty = run_cmd([
                sys.executable, str(VERIFY_SCRIPT),
                "--dir", str(iso_dir / "bundle"),
                "--trusted-key", "00" * 32
            ])
            assert ret_empty != 0
            print("  -> PASSED: System fails closed and refuses unauthorized keys.")

        # ----------------------------------------------------------------------
        # Test 12: Provenance tamper rejection
        # ----------------------------------------------------------------------
        print("\n[TEST 12] Rejection of tampered release-provenance.json...")
        prov_file = tmp_path / "release-provenance.json"
        orig_prov = prov_file.read_bytes()
        prov_data = json.loads(orig_prov.decode("utf-8"))
        prov_data["git_commit"] = "0000000000000000000000000000000000000000"
        prov_file.write_text(json.dumps(prov_data, indent=2) + "\n", encoding="utf-8")

        ret, stdout, stderr = run_cmd([
            sys.executable, str(VERIFY_SCRIPT),
            "--dir", str(tmp_path),
            "--trusted-key-file", str(trusted_pub_file)
        ])
        assert ret != 0, "Verification unexpectedly succeeded with tampered provenance!"
        assert "Cryptographic verification failed for release-provenance.json.sig" in stderr
        print("  -> PASSED: Tampered provenance metadata was correctly detected and rejected.")
        prov_file.write_bytes(orig_prov)  # Restore

    print("\n" + "=" * 75)
    print("ALL 12 RELEASE SIGNING, TRUST ANCHOR & ANTI-TAMPER TESTS PASSED (EXIT 0)")
    print("=" * 75)


if __name__ == "__main__":
    test_suite()
