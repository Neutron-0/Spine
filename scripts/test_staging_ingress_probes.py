#!/usr/bin/env python3
"""
Unit and Integration Tests for Staging Ingress Verification Prober.

Verifies:
1. FQDN placeholder and invalid hostname rejection.
2. TLS certificate chain validation and untrusted CA rejection.
3. SAN hostname mismatch rejection.
4. HTTPS strict 200 requirement (rejection of 3xx redirects and errors).
5. Mandatory security header enforcement.
6. HTTP port 80 strict 301 redirect requirement.
7. Redirect Location header matching.
8. CLI exit code fidelity.
"""

import datetime
import http.server
import ipaddress
import os
import socket
import ssl
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from typing import Optional, Tuple

import cryptography
from cryptography import x509
from cryptography.hazmat.backends import default_backend
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import NameOID

# Import functions under test
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import probe_staging_ingress


def generate_ca() -> Tuple[rsa.RSAPrivateKey, x509.Certificate, str]:
    """Generate an ephemeral CA private key and self-signed certificate."""
    ca_key = rsa.generate_private_key(public_exponent=65537, key_size=2048, backend=default_backend())
    subject = issuer = x509.Name([
        x509.NameAttribute(NameOID.COUNTRY_NAME, "US"),
        x509.NameAttribute(NameOID.ORGANIZATION_NAME, "Spine Staging Test CA"),
        x509.NameAttribute(NameOID.COMMON_NAME, "Spine Test Root CA"),
    ])
    now = datetime.datetime.now(datetime.timezone.utc)
    ski = x509.SubjectKeyIdentifier.from_public_key(ca_key.public_key())
    ca_cert = (
        x509.CertificateBuilder()
        .subject_name(subject)
        .issuer_name(issuer)
        .public_key(ca_key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(now - datetime.timedelta(days=1))
        .not_valid_after(now + datetime.timedelta(days=7))
        .add_extension(
            x509.BasicConstraints(ca=True, path_length=None),
            critical=True,
        )
        .add_extension(
            x509.KeyUsage(
                digital_signature=True,
                content_commitment=False,
                key_encipherment=False,
                data_encipherment=False,
                key_agreement=False,
                key_cert_sign=True,
                crl_sign=True,
                encipher_only=False,
                decipher_only=False,
            ),
            critical=True,
        )
        .add_extension(ski, critical=False)
        .add_extension(
            x509.AuthorityKeyIdentifier.from_issuer_public_key(ca_key.public_key()),
            critical=False,
        )
        .sign(ca_key, hashes.SHA256(), default_backend())
    )
    ca_pem = ca_cert.public_bytes(serialization.Encoding.PEM).decode("utf-8")
    return ca_key, ca_cert, ca_pem


def generate_server_cert(
    ca_key: rsa.RSAPrivateKey,
    ca_cert: x509.Certificate,
    san_dns_names: list[str],
) -> Tuple[str, str]:
    """Generate a server certificate signed by the given CA."""
    server_key = rsa.generate_private_key(public_exponent=65537, key_size=2048, backend=default_backend())
    subject = x509.Name([
        x509.NameAttribute(NameOID.COUNTRY_NAME, "US"),
        x509.NameAttribute(NameOID.ORGANIZATION_NAME, "Spine Staging Server"),
        x509.NameAttribute(NameOID.COMMON_NAME, san_dns_names[0] if san_dns_names else "localhost"),
    ])
    now = datetime.datetime.now(datetime.timezone.utc)
    ski = x509.SubjectKeyIdentifier.from_public_key(server_key.public_key())
    builder = (
        x509.CertificateBuilder()
        .subject_name(subject)
        .issuer_name(ca_cert.subject)
        .public_key(server_key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(now - datetime.timedelta(days=1))
        .not_valid_after(now + datetime.timedelta(days=2))
        .add_extension(
            x509.BasicConstraints(ca=False, path_length=None),
            critical=True,
        )
        .add_extension(
            x509.KeyUsage(
                digital_signature=True,
                content_commitment=False,
                key_encipherment=True,
                data_encipherment=False,
                key_agreement=False,
                key_cert_sign=False,
                crl_sign=False,
                encipher_only=False,
                decipher_only=False,
            ),
            critical=True,
        )
        .add_extension(
            x509.ExtendedKeyUsage([x509.oid.ExtendedKeyUsageOID.SERVER_AUTH]),
            critical=False,
        )
        .add_extension(ski, critical=False)
        .add_extension(
            x509.AuthorityKeyIdentifier.from_issuer_public_key(ca_key.public_key()),
            critical=False,
        )
    )

    alt_names = [x509.DNSName(name) for name in san_dns_names]
    builder = builder.add_extension(
        x509.SubjectAlternativeName(alt_names),
        critical=False,
    )

    cert = builder.sign(ca_key, hashes.SHA256(), default_backend())

    key_pem = server_key.private_bytes(
        encoding=serialization.Encoding.PEM,
        format=serialization.PrivateFormat.TraditionalOpenSSL,
        encryption_algorithm=serialization.NoEncryption(),
    ).decode("utf-8")
    cert_pem = cert.public_bytes(serialization.Encoding.PEM).decode("utf-8")
    return key_pem, cert_pem


class MockHttpHandler(http.server.BaseHTTPRequestHandler):
    """Configurable HTTP handler for ingress tests."""
    status_to_return = 200
    headers_to_add = {
        "Strict-Transport-Security": "max-age=31536000; includeSubDomains",
        "X-Content-Type-Options": "nosniff",
        "X-Frame-Options": "SAMEORIGIN",
        "Referrer-Policy": "strict-origin-when-cross-origin",
    }
    body_to_return = b'{"status":"healthy"}'

    def do_GET(self):
        self.send_response(self.status_to_return)
        for k, v in self.headers_to_add.items():
            self.send_header(k, v)
        self.send_header("Content-Length", str(len(self.body_to_return)))
        self.end_headers()
        self.wfile.write(self.body_to_return)

    def log_message(self, format, *args):
        # Suppress server access log in tests
        pass


class TestStagingIngressProbes(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp_dir = tempfile.TemporaryDirectory()
        cls.ca_key, cls.ca_cert, cls.ca_pem = generate_ca()

        # Save CA PEM
        cls.ca_path = os.path.join(cls.temp_dir.name, "ca.pem")
        with open(cls.ca_path, "w", encoding="utf-8") as f:
            f.write(cls.ca_pem)

        # Generate untrusted CA PEM
        _, _, untrusted_pem = generate_ca()
        cls.untrusted_ca_path = os.path.join(cls.temp_dir.name, "untrusted_ca.pem")
        with open(cls.untrusted_ca_path, "w", encoding="utf-8") as f:
            f.write(untrusted_pem)

        # Generate Valid Cert matching staging.spineforge.internal
        valid_key, valid_cert = generate_server_cert(
            cls.ca_key, cls.ca_cert, ["staging.spineforge.internal"]
        )
        cls.valid_cert_path = os.path.join(cls.temp_dir.name, "valid_cert.pem")
        cls.valid_key_path = os.path.join(cls.temp_dir.name, "valid_key.pem")
        with open(cls.valid_cert_path, "w", encoding="utf-8") as f:
            f.write(valid_cert)
        with open(cls.valid_key_path, "w", encoding="utf-8") as f:
            f.write(valid_key)

        # Generate Mismatched Cert matching other.spineforge.internal
        wrong_key, wrong_cert = generate_server_cert(
            cls.ca_key, cls.ca_cert, ["other.spineforge.internal"]
        )
        cls.wrong_cert_path = os.path.join(cls.temp_dir.name, "wrong_cert.pem")
        cls.wrong_key_path = os.path.join(cls.temp_dir.name, "wrong_key.pem")
        with open(cls.wrong_cert_path, "w", encoding="utf-8") as f:
            f.write(wrong_cert)
        with open(cls.wrong_key_path, "w", encoding="utf-8") as f:
            f.write(wrong_key)

    @classmethod
    def tearDownClass(cls):
        cls.temp_dir.cleanup()

    def test_placeholder_fqdn_validation(self):
        """Rejects empty, placeholder, or localhost domains."""
        invalid_fqdns = [
            "",
            "   ",
            "example.com",
            "foo.example.com",
            "<staging.example.com>",
            "localhost",
            "127.0.0.1",
            "0.0.0.0",
        ]
        for fqdn in invalid_fqdns:
            with self.subTest(fqdn=fqdn):
                with self.assertRaises(ValueError):
                    probe_staging_ingress.validate_fqdn(fqdn)

        # Valid domain should not raise
        probe_staging_ingress.validate_fqdn("staging.spineforge.internal")
        probe_staging_ingress.validate_fqdn("api.prod.example-corp.net")

    def test_https_probe_success(self):
        """Tests successful HTTPS probe with valid cert, status 200, and security headers."""
        server = http.server.HTTPServer(("127.0.0.1", 0), MockHttpHandler)
        port = server.server_port

        ssl_ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ssl_ctx.load_cert_chain(certfile=self.valid_cert_path, keyfile=self.valid_key_path)
        server.socket = ssl_ctx.wrap_socket(server.socket, server_side=True)

        thread = threading.Thread(target=server.handle_request)
        thread.daemon = True
        thread.start()

        headers = probe_staging_ingress.probe_https_readiness(
            hostname="staging.spineforge.internal",
            port=port,
            path="/health/ready",
            ca_file=self.ca_path,
            connect_host="127.0.0.1",
            timeout=5.0,
        )
        server.server_close()
        thread.join(timeout=2.0)

        self.assertIn("Strict-Transport-Security", headers)
        self.assertEqual(headers["X-Frame-Options"], "SAMEORIGIN")

    def test_https_probe_untrusted_ca_fails(self):
        """Fails closed when server certificate is not trusted by specified CA."""
        server = http.server.HTTPServer(("127.0.0.1", 0), MockHttpHandler)
        port = server.server_port

        ssl_ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ssl_ctx.load_cert_chain(certfile=self.valid_cert_path, keyfile=self.valid_key_path)
        server.socket = ssl_ctx.wrap_socket(server.socket, server_side=True)

        thread = threading.Thread(target=server.handle_request)
        thread.daemon = True
        thread.start()

        with self.assertRaises(ssl.SSLError):
            probe_staging_ingress.probe_https_readiness(
                hostname="staging.spineforge.internal",
                port=port,
                path="/health/ready",
                ca_file=self.untrusted_ca_path,  # Untrusted CA!
                connect_host="127.0.0.1",
                timeout=5.0,
            )
        server.server_close()
        thread.join(timeout=2.0)

    def test_https_probe_san_mismatch_fails(self):
        """Fails closed when certificate SAN does not match hostname."""
        server = http.server.HTTPServer(("127.0.0.1", 0), MockHttpHandler)
        port = server.server_port

        ssl_ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ssl_ctx.load_cert_chain(certfile=self.wrong_cert_path, keyfile=self.wrong_key_path)
        server.socket = ssl_ctx.wrap_socket(server.socket, server_side=True)

        thread = threading.Thread(target=server.handle_request)
        thread.daemon = True
        thread.start()

        with self.assertRaises(ssl.CertificateError):
            probe_staging_ingress.probe_https_readiness(
                hostname="staging.spineforge.internal",
                port=port,
                path="/health/ready",
                ca_file=self.ca_path,
                connect_host="127.0.0.1",
                timeout=5.0,
            )
        server.server_close()
        thread.join(timeout=2.0)

    def test_https_probe_unexpected_status_fails(self):
        """Rejects non-200 responses including 3xx redirects on the HTTPS readiness endpoint."""
        class RedirectHttpHandler(MockHttpHandler):
            status_to_return = 302
            headers_to_add = {"Location": "https://staging.spineforge.internal/login"}

        server = http.server.HTTPServer(("127.0.0.1", 0), RedirectHttpHandler)
        port = server.server_port

        ssl_ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ssl_ctx.load_cert_chain(certfile=self.valid_cert_path, keyfile=self.valid_key_path)
        server.socket = ssl_ctx.wrap_socket(server.socket, server_side=True)

        thread = threading.Thread(target=server.handle_request)
        thread.daemon = True
        thread.start()

        with self.assertRaises(RuntimeError) as cm:
            probe_staging_ingress.probe_https_readiness(
                hostname="staging.spineforge.internal",
                port=port,
                path="/health/ready",
                ca_file=self.ca_path,
                connect_host="127.0.0.1",
                timeout=5.0,
            )
        self.assertIn("Expected HTTP 200 OK", str(cm.exception))
        server.server_close()
        thread.join(timeout=2.0)

    def test_https_probe_missing_security_headers_fails(self):
        """Fails closed when required security headers are absent."""
        class InsecureHttpHandler(MockHttpHandler):
            status_to_return = 200
            headers_to_add = {
                # Missing Strict-Transport-Security and Referrer-Policy
                "X-Content-Type-Options": "nosniff",
                "X-Frame-Options": "SAMEORIGIN",
            }

        server = http.server.HTTPServer(("127.0.0.1", 0), InsecureHttpHandler)
        port = server.server_port

        ssl_ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ssl_ctx.load_cert_chain(certfile=self.valid_cert_path, keyfile=self.valid_key_path)
        server.socket = ssl_ctx.wrap_socket(server.socket, server_side=True)

        thread = threading.Thread(target=server.handle_request)
        thread.daemon = True
        thread.start()

        with self.assertRaises(RuntimeError) as cm:
            probe_staging_ingress.probe_https_readiness(
                hostname="staging.spineforge.internal",
                port=port,
                path="/health/ready",
                ca_file=self.ca_path,
                connect_host="127.0.0.1",
                timeout=5.0,
            )
        self.assertIn("security header validation failed", str(cm.exception))
        self.assertIn("Strict-Transport-Security", str(cm.exception))
        server.server_close()
        thread.join(timeout=2.0)

    def test_http_redirection_probe_success(self):
        """Passes when HTTP port returns strictly 301 and canonical HTTPS Location."""
        class RedirectHandler(MockHttpHandler):
            status_to_return = 301
            headers_to_add = {"Location": "https://staging.spineforge.internal:8443/health/ready"}

        server = http.server.HTTPServer(("127.0.0.1", 0), RedirectHandler)
        port = server.server_port

        thread = threading.Thread(target=server.handle_request)
        thread.daemon = True
        thread.start()

        loc = probe_staging_ingress.probe_http_redirection(
            hostname="staging.spineforge.internal",
            port=port,
            https_port=8443,
            path="/health/ready",
            connect_host="127.0.0.1",
            timeout=5.0,
        )
        server.server_close()
        thread.join(timeout=2.0)
        self.assertEqual(loc, "https://staging.spineforge.internal:8443/health/ready")

    def test_http_redirection_wrong_status_fails(self):
        """Fails when HTTP port returns 200 instead of 301."""
        server = http.server.HTTPServer(("127.0.0.1", 0), MockHttpHandler)
        port = server.server_port

        thread = threading.Thread(target=server.handle_request)
        thread.daemon = True
        thread.start()

        with self.assertRaises(RuntimeError) as cm:
            probe_staging_ingress.probe_http_redirection(
                hostname="staging.spineforge.internal",
                port=port,
                https_port=443,
                path="/health/ready",
                connect_host="127.0.0.1",
                timeout=5.0,
            )
        self.assertIn("Expected HTTP 301", str(cm.exception))
        server.server_close()
        thread.join(timeout=2.0)

    def test_http_redirection_wrong_location_fails(self):
        """Fails when Location header does not match canonical HTTPS target."""
        class WrongLocationHandler(MockHttpHandler):
            status_to_return = 301
            headers_to_add = {"Location": "http://insecure-redirect.com/health/ready"}

        server = http.server.HTTPServer(("127.0.0.1", 0), WrongLocationHandler)
        port = server.server_port

        thread = threading.Thread(target=server.handle_request)
        thread.daemon = True
        thread.start()

        with self.assertRaises(RuntimeError) as cm:
            probe_staging_ingress.probe_http_redirection(
                hostname="staging.spineforge.internal",
                port=port,
                https_port=443,
                path="/health/ready",
                connect_host="127.0.0.1",
                timeout=5.0,
            )
        self.assertIn("Location mismatch", str(cm.exception))
        server.server_close()
        thread.join(timeout=2.0)

    def test_cli_invocation_placeholder_fails(self):
        """CLI invocation with placeholder exits non-zero."""
        res = subprocess.run(
            [sys.executable, "scripts/probe_staging_ingress.py", "--fqdn", "example.com"],
            capture_output=True,
            text=True,
        )
        self.assertNotEqual(res.returncode, 0)
        self.assertIn("placeholder", res.stderr)

    def test_cli_invocation_success(self):
        """CLI invocation runs full probes and exits zero on valid endpoint."""
        class CombinedHandler(MockHttpHandler):
            pass

        server_https = http.server.HTTPServer(("127.0.0.1", 0), CombinedHandler)
        https_port = server_https.server_port
        ssl_ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ssl_ctx.load_cert_chain(certfile=self.valid_cert_path, keyfile=self.valid_key_path)
        server_https.socket = ssl_ctx.wrap_socket(server_https.socket, server_side=True)

        class RedirectPortHandler(MockHttpHandler):
            status_to_return = 301
            headers_to_add = {"Location": f"https://staging.spineforge.internal:{https_port}/health/ready"}

        server_http = http.server.HTTPServer(("127.0.0.1", 0), RedirectPortHandler)
        http_port = server_http.server_port

        t_https = threading.Thread(target=server_https.handle_request)
        t_http = threading.Thread(target=server_http.handle_request)
        t_https.daemon = True
        t_http.daemon = True
        t_https.start()
        t_http.start()

        res = subprocess.run(
            [
                sys.executable,
                "scripts/probe_staging_ingress.py",
                "--fqdn",
                "staging.spineforge.internal",
                "--port-https",
                str(https_port),
                "--port-http",
                str(http_port),
                "--ca-file",
                self.ca_path,
                "--connect-ip",
                "127.0.0.1",
            ],
            capture_output=True,
            text=True,
        )
        server_https.server_close()
        server_http.server_close()
        t_https.join(timeout=2.0)
        t_http.join(timeout=2.0)

        self.assertEqual(res.returncode, 0, f"STDOUT:\n{res.stdout}\nSTDERR:\n{res.stderr}")
        self.assertIn("ALL INGRESS PROBES PASSED SUCCESSFULLY", res.stdout)


if __name__ == "__main__":
    unittest.main()
