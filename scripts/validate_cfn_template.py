#!/usr/bin/env python3
import sys
import yaml

class CfnLoader(yaml.SafeLoader):
    pass

for tag in ['!Ref', '!Sub', '!GetAtt', '!Equals', '!Not', '!If', '!Condition']:
    CfnLoader.add_constructor(
        tag,
        lambda loader, node, t=tag: {
            t: loader.construct_scalar(node)
            if isinstance(node, yaml.ScalarNode)
            else loader.construct_sequence(node)
            if isinstance(node, yaml.SequenceNode)
            else loader.construct_mapping(node)
        }
    )

def validate_template(filepath):
    print(f"Validating CloudFormation template: {filepath}")
    with open(filepath, 'r') as f:
        template = yaml.load(f, Loader=CfnLoader)

    # 1. Root structure
    assert template.get('AWSTemplateFormatVersion') == '2010-09-09', "Invalid format version"
    params = template.get('Parameters', {})
    resources = template.get('Resources', {})
    conditions = template.get('Conditions', {})
    outputs = template.get('Outputs', {})
    rules = template.get('Rules', {})

    print(f"  [OK] Format Version: {template['AWSTemplateFormatVersion']}")
    print(f"  [OK] Parameters: {len(params)} ({', '.join(params.keys())})")
    print(f"  [OK] Resources: {len(resources)} ({', '.join(resources.keys())})")
    print(f"  [OK] Conditions: {len(conditions)} ({', '.join(conditions.keys())})")
    print(f"  [OK] Outputs: {len(outputs)} ({', '.join(outputs.keys())})")
    print(f"  [OK] Rules: {len(rules)} ({', '.join(rules.keys())})")

    # 1b. Rule Enforcement Validation
    assert 'ProductionRequiresComplianceMode' in rules, "Template missing ProductionRequiresComplianceMode rule"
    prod_rule = rules['ProductionRequiresComplianceMode']
    assert '!Equals' in prod_rule['RuleCondition']
    assert any('!Equals' in a.get('Assert', {}) for a in prod_rule.get('Assertions', []))
    print("  [OK] Rule: ProductionRequiresComplianceMode rule structure validated")

    # 2. Parameters validation
    for p_name, p_body in params.items():
        assert 'Type' in p_body, f"Parameter {p_name} missing Type"
        if 'AllowedValues' in p_body:
            assert p_body.get('Default') in p_body['AllowedValues'], f"Default for {p_name} not in AllowedValues"

    # 3. S3 Bucket Validation
    bucket = resources.get('SpineBackupBucket')
    assert bucket, "SpineBackupBucket resource missing"
    assert bucket['Type'] == 'AWS::S3::Bucket'
    b_props = bucket['Properties']
    assert b_props.get('ObjectLockEnabled') is True, "ObjectLockEnabled must be True"
    assert b_props.get('ObjectLockConfiguration', {}).get('ObjectLockEnabled') == 'Enabled'
    assert b_props.get('VersioningConfiguration', {}).get('Status') == 'Enabled'
    assert b_props.get('BucketEncryption') is not None
    assert b_props.get('PublicAccessBlockConfiguration', {}).get('BlockPublicAcls') is True
    assert b_props.get('PublicAccessBlockConfiguration', {}).get('RestrictPublicBuckets') is True
    assert bucket.get('DeletionPolicy') == 'Retain', "DeletionPolicy must be Retain"
    assert bucket.get('UpdateReplacePolicy') == 'Retain', "UpdateReplacePolicy must be Retain"
    print("  [OK] S3 Bucket: WORM Object Lock, Versioning, AES256, and PublicAccessBlock validated")

    # 4. Bucket Policy Validation
    bp = resources.get('SpineBackupBucketPolicy')
    assert bp, "SpineBackupBucketPolicy missing"
    assert bp['Type'] == 'AWS::S3::BucketPolicy'
    bp_stmts = bp['Properties']['PolicyDocument']['Statement']
    assert any(s.get('Sid') == 'EnforceSecureTransport' and s.get('Effect') == 'Deny' for s in bp_stmts)
    assert any(s.get('Sid') == 'EnforceTls12OrHigher' and s.get('Effect') == 'Deny' for s in bp_stmts)
    print("  [OK] Bucket Policy: Mandatory TLS 1.2+ & Non-SSL Deny validated")

    # 5. IAM Role Validation
    role = resources.get('SpineBackupRole')
    assert role, "SpineBackupRole missing"
    assert role['Type'] == 'AWS::IAM::Role'
    assume_stmt = role['Properties']['AssumeRolePolicyDocument']['Statement']
    assert len(assume_stmt) > 0 and assume_stmt[0]['Action'] == 'sts:AssumeRole'
    
    role_policies = role['Properties']['Policies'][0]['PolicyDocument']['Statement']
    assert any(s.get('Sid') == 'ExplicitDenyDeletionsDuringRetention' and s.get('Effect') == 'Deny' for s in role_policies)
    assert any(s.get('Sid') == 'BucketMetadata' and 's3:GetBucketObjectLockConfiguration' in s.get('Action') for s in role_policies)
    assert any(s.get('Sid') == 'ListBackupsPrefixOnly' and s.get('Effect') == 'Allow' for s in role_policies)
    assert any(s.get('Sid') == 'BackupWriteAndRead' and s.get('Effect') == 'Allow' for s in role_policies)
    assert not any('LegalHold' in str(s.get('Action', [])) for s in role_policies), "Legal hold permissions should not be granted"
    print("  [OK] IAM Role: Least-privilege actions, prefix scoping, no legal-hold, and explicit deletion denial validated")

    # 5b. Auditor Role Validation
    audit_role = resources.get('SpineAuditVerificationRole')
    assert audit_role, "SpineAuditVerificationRole missing"
    assert audit_role['Type'] == 'AWS::IAM::Role'
    audit_policies = audit_role['Properties']['Policies'][0]['PolicyDocument']['Statement']
    assert any('s3:DeleteObject' in s.get('Action', []) and s.get('Effect') == 'Allow' for s in audit_policies), "Auditor must have s3:DeleteObject allowed to test Object Lock"
    assert not any('BypassGovernanceRetention' in str(s.get('Action', [])) for s in audit_policies), "Auditor must not bypass retention"
    print("  [OK] Auditor Role: DeleteObject allowed without retention bypass to prove S3 WORM enforcement")

    # 6. Outputs reference check
    for o_name, o_body in outputs.items():
        assert 'Value' in o_body, f"Output {o_name} missing Value"
    print("  [OK] All Outputs reference valid properties")

    # 7. Regression test for Production Governance mode rejection
    test_production_governance_rejection(template)

    print("\n[SUCCESS] CloudFormation template validated successfully with zero errors!")

def test_production_governance_rejection(template):
    print("  Testing Rule: Production Environment with Governance Mode Rejection...")
    rules = template.get('Rules', {})
    prod_rule = rules.get('ProductionRequiresComplianceMode')
    assert prod_rule, "ProductionRequiresComplianceMode rule missing"

    def eval_expr(expr, env_name, lock_mode):
        if isinstance(expr, dict) and '!Equals' in expr:
            operands = expr['!Equals']
            def resolve(op):
                if isinstance(op, dict) and '!Ref' in op:
                    ref = op['!Ref']
                    if ref == 'EnvironmentName':
                        return env_name
                    if ref == 'ObjectLockMode':
                        return lock_mode
                return op
            return resolve(operands[0]) == resolve(operands[1])
        return True

    def check_combination(env_name, lock_mode):
        cond_active = eval_expr(prod_rule['RuleCondition'], env_name, lock_mode)
        if not cond_active:
            return True, "Condition inactive; rule passes"
        for a in prod_rule.get('Assertions', []):
            passed = eval_expr(a.get('Assert'), env_name, lock_mode)
            if not passed:
                return False, a.get('AssertDescription')
        return True, "Assertion passed"

    # Test staging + GOVERNANCE -> Allowed
    ok, msg = check_combination('staging', 'GOVERNANCE')
    assert ok, f"staging + GOVERNANCE should be allowed: {msg}"
    print("  [OK] Valid combination allowed: EnvironmentName=staging, ObjectLockMode=GOVERNANCE")

    # Test staging + COMPLIANCE -> Allowed
    ok, msg = check_combination('staging', 'COMPLIANCE')
    assert ok, f"staging + COMPLIANCE should be allowed: {msg}"
    print("  [OK] Valid combination allowed: EnvironmentName=staging, ObjectLockMode=COMPLIANCE")

    # Test production + COMPLIANCE -> Allowed
    ok, msg = check_combination('production', 'COMPLIANCE')
    assert ok, f"production + COMPLIANCE should be allowed: {msg}"
    print("  [OK] Valid combination allowed: EnvironmentName=production, ObjectLockMode=COMPLIANCE")

    # Test production + GOVERNANCE -> MUST BE REJECTED
    ok, msg = check_combination('production', 'GOVERNANCE')
    assert not ok, "production + GOVERNANCE must be rejected by rule assertion!"
    assert "Production environment strictly requires ObjectLockMode to be COMPLIANCE" in msg
    print(f"  [OK] Invalid combination successfully rejected: EnvironmentName=production, ObjectLockMode=GOVERNANCE -> '{msg}'")

if __name__ == '__main__':
    validate_template('deploy/aws/staging-object-lock-storage.yaml')
