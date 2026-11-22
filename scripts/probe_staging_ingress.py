#!/usr/bin/env python3
"""
Spine Forge - Staging Ingress Verification Prober

Executes robust HTTPS readiness and plain HTTP redirection verification
against an authoritative staging domain (${STAGING_FQDN}).

Requirements enforced:
1. Rejects placeholder or localhost hostnames.
2. Probe 4a (HTTPS):
   - Validates TLS certificate chain against system/specified trust store without insecure flags.
   - Validates that certificate SAN explicitly matches ${STAGING_FQDN}.
   - Issues a GET request with redirects disabled.
   - Asserts strictly HTTP 200 OK (rejecting 3xx redirects and 4xx/5xx errors).
   - Asserts mandatory security headers: Strict-Transport-Security, X-Content-Type-Options,
     X-Frame-Options, Referrer-Policy.
3. Probe 4b (HTTP):
   - Issues a GET request against port 80 with redirects disabled.
   - Asserts strictly HTTP 301 Moved Permanently.
   - Asserts Location header matches https://${STAGING_FQDN}/health/ready.
"""

import argparse
import http.client
import re
import socket
import ssl
import sys
import urllib.parse
from typing import Dict, List, Optional, Tuple


PLACEHOLDER_PATTERN = re.compile(
    r"^(example\.com|.*\.example\.com|<.*>|localhost|127\.0\.0\.1|0\.0\.0\.0)$",
    re.IGNORECASE,
)

MANDATORY_SECURITY_HEADERS = [
    ("Strict-Transport-Security", r"max-age=\d+"),
    ("X-Content-Type-Options", r"nosniff"),
    ("X-Frame-Options", r"SAMEORIGIN"),
    ("Referrer-Policy", r"strict-origin-when-cross-origin"),
]


def validate_fqdn(fqdn: str) -> None:
    """Ensure FQDN is non-empty and not a placeholder."""
    if not fqdn or not fqdn.strip():
        raise ValueError("STAGING_FQDN must not be empty.")
    fqdn_clean = fqdn.strip()
    if PLACEHOLDER_PATTERN.match(fqdn_clean):
        raise ValueError(
            f"STAGING_FQDN '{fqdn}' is a placeholder or localhost. "
            "A valid, resolvable public staging domain is required."
        )


def verify_tls_certificate(
    hostname: str,
    port: int = 443,
    ca_file: Optional[str] = None,
    timeout: float = 10.0,
    connect_host: Optional[str] = None,
) -> ssl.SSLSocket:
    """
    Establish a TLS connection verifying root CA chain trust and hostname matching.
    Fails closed on untrusted CA, expired cert, or hostname mismatch.
    """
    context = ssl.create_default_context(cafile=ca_file)
    context.check_hostname = True
    context.verify_mode = ssl.CERT_REQUIRED

    # Enforce TLS 1.2 or TLS 1.3
    context.minimum_version = ssl.TLSVersion.TLSv1_2

    target_host = connect_host or hostname
    sock = socket.create_connection((target_host, port), timeout=timeout)
    ssock = context.wrap_socket(sock, server_hostname=hostname)

    cert = ssock.getpeercert()
    if not cert:
        raise ssl.SSLError(f"No peer certificate presented by {hostname}:{port}")

    # Verify Subject Alternative Name explicitly includes hostname
    san_entries = cert.get("subjectAltName", ())
    dns_sans = [entry[1] for entry in san_entries if entry[0] == "DNS"]

    if hostname not in dns_sans:
        # Check wildcard matches
        matched = False
        for san in dns_sans:
            if san.startswith("*.") and hostname.endswith(san[1:]):
                matched = True
                break
        if not matched:
            raise ssl.CertificateError(
                f"Hostname mismatch: '{hostname}' not found in certificate SANs: {dns_sans}"
            )

    return ssock


def probe_https_readiness(
    hostname: str,
    port: int = 443,
    path: str = "/health/ready",
    ca_file: Optional[str] = None,
    timeout: float = 10.0,
    connect_host: Optional[str] = None,
) -> Dict[str, str]:
    """
    Execute HTTPS probe against canonical path.
    Asserts strictly HTTP 200 and required security headers without following redirects.
    """
    # 1. Verify TLS handshake and certificate SAN
    ssock = verify_tls_certificate(
        hostname, port, ca_file=ca_file, timeout=timeout, connect_host=connect_host
    )

    # 2. Issue HTTP/1.1 GET request over the verified TLS socket
    conn = http.client.HTTPSConnection(
        hostname,
        port=port,
        context=ssock.context,
        timeout=timeout,
    )
    # Re-use socket
    conn.sock = ssock

    conn.request("GET", path, headers={"Host": hostname, "User-Agent": "Spine-Staging-Prober/1.0"})
    resp = conn.getresponse()
    headers = {k.title(): v for k, v in resp.getheaders()}

    if resp.status != 200:
        raise RuntimeError(
            f"HTTPS probe failed: Expected HTTP 200 OK from https://{hostname}:{port}{path}, "
            f"got HTTP {resp.status} {resp.reason}. Redirects are not permitted on the readiness endpoint."
        )

    # 3. Assert mandatory security headers
    missing_or_invalid: List[str] = []
    for header_name, pattern in MANDATORY_SECURITY_HEADERS:
        val = headers.get(header_name.title())
        if not val or not re.search(pattern, val, re.IGNORECASE):
            missing_or_invalid.append(f"{header_name} (expected match for '{pattern}', got '{val}')")

    if missing_or_invalid:
        raise RuntimeError(
            f"HTTPS probe security header validation failed for {hostname}: "
            + "; ".join(missing_or_invalid)
        )

    return headers


def probe_http_redirection(
    hostname: str,
    port: int = 80,
    https_port: int = 443,
    path: str = "/health/ready",
    timeout: float = 10.0,
    connect_host: Optional[str] = None,
) -> str:
    """
    Execute plain HTTP probe against port 80.
    Asserts strictly HTTP 301 and Location matching canonical HTTPS URL.
    """
    target_host = connect_host or hostname
    conn = http.client.HTTPConnection(target_host, port=port, timeout=timeout)
    conn.request("GET", path, headers={"Host": hostname, "User-Agent": "Spine-Staging-Prober/1.0"})
    resp = conn.getresponse()
    headers = {k.title(): v for k, v in resp.getheaders()}

    if resp.status != 301:
        raise RuntimeError(
            f"HTTP redirection probe failed: Expected HTTP 301 Moved Permanently from "
            f"http://{hostname}:{port}{path}, got HTTP {resp.status} {resp.reason}."
        )

    location = headers.get("Location")
    if not location:
        raise RuntimeError("HTTP 301 response is missing required 'Location' header.")

    # Expected location
    if https_port == 443:
        expected_prefix = f"https://{hostname}"
    else:
        expected_prefix = f"https://{hostname}:{https_port}"
    expected_location = f"{expected_prefix}{path}"

    if location != expected_location:
        raise RuntimeError(
            f"HTTP redirect Location mismatch: expected '{expected_location}', got '{location}'"
        )

    return location


def run_all_probes(
    fqdn: str,
    port_https: int = 443,
    port_http: int = 80,
    path: str = "/health/ready",
    ca_file: Optional[str] = None,
    timeout: float = 10.0,
    connect_host: Optional[str] = None,
) -> bool:
    """Run both probes and print diagnostic output."""
    print(f"=== Spine Forge Staging Ingress Verification for [{fqdn}] ===")
    validate_fqdn(fqdn)
    print(f"1. Target FQDN validated: {fqdn}")

    # Probe 4a: HTTPS Readiness Probe
    print(f"2. Executing Probe 4a (HTTPS port {port_https}, path {path})...")
    https_headers = probe_https_readiness(
        fqdn, port=port_https, path=path, ca_file=ca_file, timeout=timeout, connect_host=connect_host
    )
    print("   [PASS] TLS Handshake: Verified certificate chain trust and SAN match.")
    print("   [PASS] HTTP Status: Exactly 200 OK.")
    for hname, _ in MANDATORY_SECURITY_HEADERS:
        print(f"   [PASS] Security Header: {hname} = {https_headers.get(hname.title())}")

    # Probe 4b: Plain HTTP Redirection Probe
    print(f"3. Executing Probe 4b (HTTP port {port_http}, path {path})...")
    redirect_loc = probe_http_redirection(
        fqdn,
        port=port_http,
        https_port=port_https,
        path=path,
        timeout=timeout,
        connect_host=connect_host,
    )
    print("   [PASS] HTTP Status: Exactly 301 Moved Permanently.")
    print(f"   [PASS] Redirect Target: Strictly '{redirect_loc}'.")

    print("\n=== ALL INGRESS PROBES PASSED SUCCESSFULLY ===")
    return True


def main() -> None:
    parser = argparse.ArgumentParser(description="Spine Forge Ingress Verification Prober")
    parser.add_argument("--fqdn", required=True, help="Authoritative staging domain name")
    parser.add_argument("--port-https", type=int, default=443, help="HTTPS port (default: 443)")
    parser.add_argument("--port-http", type=int, default=80, help="Plain HTTP port (default: 80)")
    parser.add_argument("--path", default="/health/ready", help="Canonical readiness path")
    parser.add_argument("--ca-file", default=None, help="Optional CA bundle for local testing")
    parser.add_argument("--connect-ip", default=None, help="Optional IP address override (e.g. for testing)")
    parser.add_argument("--timeout", type=float, default=10.0, help="Socket timeout seconds")

    args = parser.parse_args()

    try:
        run_all_probes(
            fqdn=args.fqdn,
            port_https=args.port_https,
            port_http=args.port_http,
            path=args.path,
            ca_file=args.ca_file,
            timeout=args.timeout,
            connect_host=args.connect_ip,
        )
    except Exception as exc:
        print(f"\n[FATAL ERROR] Ingress verification failed: {exc}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
