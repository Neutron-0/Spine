#!/usr/bin/env bash
# ==============================================================================
# Spine Forge - Staging Ingress Verification Prober (POSIX Bash Wrapper)
# ==============================================================================
set -euo pipefail

STAGING_FQDN="${1:-${STAGING_FQDN:-}}"
HTTPS_PORT="${2:-${STAGING_HTTPS_PORT:-443}}"
HTTP_PORT="${3:-${STAGING_HTTP_PORT:-80}}"
READINESS_PATH="${4:-/health/ready}"
CA_FILE="${5:-${STAGING_CA_FILE:-}}"
CONNECT_IP="${6:-${STAGING_CONNECT_IP:-}}"

if [ -z "${STAGING_FQDN}" ]; then
  echo "Usage: $0 <STAGING_FQDN> [HTTPS_PORT] [HTTP_PORT] [READINESS_PATH] [CA_FILE] [CONNECT_IP]" >&2
  exit 1
fi

# 1. Validate hostname
if [[ "${STAGING_FQDN}" =~ ^(example\.com|.*\.example\.com|<.*>|localhost|127\.0\.0\.1|0\.0\.0\.0)$ ]]; then
  echo "FATAL: STAGING_FQDN '${STAGING_FQDN}' is a placeholder or localhost. Real public domain required." >&2
  exit 1
fi

echo "=== Spine Forge Staging Ingress Probe for [${STAGING_FQDN}] ==="

# 2. Probe 4a: HTTPS Readiness Probe
echo "--- Running Probe 4a: HTTPS Readiness on port ${HTTPS_PORT} ---"
OPENSSL_EXTRA_ARGS=()
if [ -n "${CA_FILE}" ]; then
  OPENSSL_EXTRA_ARGS+=(-CAfile "${CA_FILE}")
fi

# Validate full TLS certificate chain and hostname matching via OpenSSL
CONNECT_TARGET="${STAGING_FQDN}"
CURL_RESOLVE_ARGS=()
if [ -n "${CONNECT_IP}" ]; then
  CONNECT_TARGET="${CONNECT_IP}"
  CURL_RESOLVE_ARGS+=(--resolve "${STAGING_FQDN}:${HTTPS_PORT}:${CONNECT_IP}")
fi

set +e
TLS_VERIFY_OUTPUT=$(echo | openssl s_client -connect "${CONNECT_TARGET}:${HTTPS_PORT}" -servername "${STAGING_FQDN}" -verify_return_error "${OPENSSL_EXTRA_ARGS[@]}" 2>&1)
TLS_VERIFY_EXIT=$?
set -e

if [ ${TLS_VERIFY_EXIT} -ne 0 ]; then
  echo "FATAL: OpenSSL certificate chain verification failed for ${STAGING_FQDN}:${HTTPS_PORT}!" >&2
  echo "${TLS_VERIFY_OUTPUT}" >&2
  exit 1
fi

# Assert SAN match
LIVE_SAN=$(echo "${TLS_VERIFY_OUTPUT}" | openssl x509 -noout -text 2>/dev/null \
  | grep -A1 "Subject Alternative Name" \
  | grep -o "DNS:${STAGING_FQDN}" || true)

if [ -z "${LIVE_SAN}" ]; then
  echo "FATAL: Live certificate SAN does not match DNS:${STAGING_FQDN}!" >&2
  exit 1
fi
echo "   [PASS] TLS Handshake: Certificate chain and SAN validated."

# Query endpoint over HTTPS with redirects disabled
CURL_CA_ARGS=()
if [ -n "${CA_FILE}" ]; then
  CURL_CA_ARGS+=(--cacert "${CA_FILE}")
fi

HTTPS_HEADER_DUMP=$(curl -s -S -D - -o /dev/null "${CURL_CA_ARGS[@]}" "${CURL_RESOLVE_ARGS[@]}" "https://${STAGING_FQDN}:${HTTPS_PORT}${READINESS_PATH}")
HTTPS_STATUS_LINE=$(echo "${HTTPS_HEADER_DUMP}" | grep -E "^HTTP/[12\.]+" | tail -n 1)
HTTPS_STATUS_CODE=$(echo "${HTTPS_STATUS_LINE}" | awk '{print $2}')

if [ "${HTTPS_STATUS_CODE}" != "200" ]; then
  echo "FATAL: HTTPS readiness probe did not return HTTP 200! Received: '${HTTPS_STATUS_LINE}'" >&2
  echo "${HTTPS_HEADER_DUMP}" >&2
  exit 1
fi
echo "   [PASS] HTTP Status: Exactly 200 OK."

# Assert mandatory security headers
for header_pattern in "Strict-Transport-Security:.*max-age=" "X-Content-Type-Options: nosniff" "X-Frame-Options: SAMEORIGIN" "Referrer-Policy: strict-origin-when-cross-origin"; do
  if ! echo "${HTTPS_HEADER_DUMP}" | grep -qi "^${header_pattern}"; then
    echo "FATAL: Missing or invalid security header: ${header_pattern}" >&2
    exit 1
  fi
done
echo "   [PASS] Security Headers: All 4 mandatory headers verified."

# 3. Probe 4b: Plain HTTP Redirection Probe
echo "--- Running Probe 4b: HTTP Redirection on port ${HTTP_PORT} ---"
HTTP_RESOLVE_ARGS=()
if [ -n "${CONNECT_IP}" ]; then
  HTTP_RESOLVE_ARGS+=(--resolve "${STAGING_FQDN}:${HTTP_PORT}:${CONNECT_IP}")
fi
HTTP_HEADER_DUMP=$(curl -s -S -D - -o /dev/null "${HTTP_RESOLVE_ARGS[@]}" "http://${STAGING_FQDN}:${HTTP_PORT}${READINESS_PATH}")

if ! echo "${HTTP_HEADER_DUMP}" | grep -qE "^HTTP/[12\.]+ 301"; then
  echo "FATAL: Port ${HTTP_PORT} did not return HTTP 301! Full headers:" >&2
  echo "${HTTP_HEADER_DUMP}" >&2
  exit 1
fi
echo "   [PASS] HTTP Status: Exactly 301 Moved Permanently."

LOCATION_HEADER=$(echo "${HTTP_HEADER_DUMP}" | grep -i "^Location:" | tr -d '\r' | awk '{print $2}')
if [ "${HTTPS_PORT}" = "443" ]; then
  EXPECTED_LOCATION="https://${STAGING_FQDN}${READINESS_PATH}"
else
  EXPECTED_LOCATION="https://${STAGING_FQDN}:${HTTPS_PORT}${READINESS_PATH}"
fi

if [ "${LOCATION_HEADER}" != "${EXPECTED_LOCATION}" ]; then
  echo "FATAL: Location header '${LOCATION_HEADER}' does not match expected '${EXPECTED_LOCATION}'!" >&2
  exit 1
fi
echo "   [PASS] Redirect Target: Strictly '${EXPECTED_LOCATION}'."

echo "=== ALL INGRESS PROBES PASSED SUCCESSFULLY ==="
