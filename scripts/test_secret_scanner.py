#!/usr/bin/env python3
"""
Test suite for secret scanning engine.
Verifies that:
1. Synthetic tokens (sp_pat_*, AKIA*) trigger detection.
2. Findings are strictly redacted in report output (zero raw secret disclosure).
3. Documented mock examples (AKIAIOSFODNN7EXAMPLE) are properly whitelisted.
"""

import re
import sys

# Secret detection patterns (matching .gitleaks.toml)
PATTERNS = [
    ("spine_pat", re.compile(r"(?i)(?:sp_pat_|spine_pat_)[a-zA-Z0-9_-]{20,}")),
    ("aws_access_key", re.compile(r"\b(AKIA[0-9A-Z]{16})\b")),
]

ALLOWLIST = [
    re.compile(r"AKIAIOSFODNN7EXAMPLE"),
]

def scan_text_safe(content: str) -> list[dict]:
    """
    Scans text for secrets and returns sanitized, redacted findings.
    Never prints or stores raw secret tokens.
    """
    findings = []
    for line_num, line in enumerate(content.splitlines(), start=1):
        for rule_id, pattern in PATTERNS:
            for match in pattern.finditer(line):
                raw_token = match.group(0)
                # Check allowlist
                if any(allow.search(raw_token) for allow in ALLOWLIST):
                    continue
                # Redact finding
                redacted_token = raw_token[:6] + "..." + raw_token[-4:] if len(raw_token) > 10 else "[REDACTED]"
                findings.append({
                    "rule": rule_id,
                    "line": line_num,
                    "redacted": redacted_token,
                })
    return findings

def test_synthetic_secret_detection():
    # 1. Synthetic PAT
    synthetic_pat = "sp_pat_synthetic_test_token_abcdef123456"
    test_data = f'let token = "{synthetic_pat}";\n'
    findings = scan_text_safe(test_data)
    assert len(findings) == 1, f"Expected 1 finding, got {len(findings)}"
    assert findings[0]["rule"] == "spine_pat"
    # Verify raw token is NOT in redacted output
    assert synthetic_pat not in findings[0]["redacted"]
    assert findings[0]["redacted"].startswith("sp_pat")
    print("  [PASS] Synthetic PAT detected and safely redacted without disclosure.")

    # 2. Synthetic AWS Key
    synthetic_aws = "AKIA1234567890ABCDEF"
    test_data_aws = f'export AWS_KEY="{synthetic_aws}"\n'
    findings_aws = scan_text_safe(test_data_aws)
    assert len(findings_aws) == 1, f"Expected 1 finding, got {len(findings_aws)}"
    assert findings_aws[0]["rule"] == "aws_access_key"
    assert synthetic_aws not in findings_aws[0]["redacted"]
    print("  [PASS] Synthetic AWS Key detected and safely redacted without disclosure.")

    # 3. Allowlisted standard test oracle
    mock_oracle = "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE\n"
    findings_oracle = scan_text_safe(mock_oracle)
    assert len(findings_oracle) == 0, f"Allowlisted oracle should not trigger findings: {findings_oracle}"
    print("  [PASS] Standard mock oracle AKIAIOSFODNN7EXAMPLE properly whitelisted.")

    print("\n[SUCCESS] Secret scanner safety test passed with zero raw token leaks!")

if __name__ == "__main__":
    test_synthetic_secret_detection()
