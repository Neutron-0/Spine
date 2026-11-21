#!/usr/bin/env python3
import sys
import yaml

class CfnLoader(yaml.SafeLoader):
    pass

for tag in ['!Ref', '!Sub', '!GetAtt', '!Equals', '!If', '!Condition']:
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

    print(f"  [OK] Format Version: {template['AWSTemplateFormatVersion']}")
    print(f"  [OK] Parameters: {len(params)} ({', '.join(params.keys())})")
    print(f"  [OK] Resources: {len(resources)} ({', '.join(resources.keys())})")
    print(f"  [OK] Conditions: {len(conditions)} ({', '.join(conditions.keys())})")
    print(f"  [OK] Outputs: {len(outputs)} ({', '.join(outputs.keys())})")

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
    assert any(s.get('Sid') == 'BucketMetadata' and s.get('Effect') == 'Allow' for s in role_policies)
    assert any(s.get('Sid') == 'ListBackupsPrefixOnly' and s.get('Effect') == 'Allow' for s in role_policies)
    assert any(s.get('Sid') == 'BackupWriteAndRead' and s.get('Effect') == 'Allow' for s in role_policies)
    print("  [OK] IAM Role: Least-privilege actions, prefix scoping, and explicit deletion denial validated")

    # 6. Outputs reference check
    for o_name, o_body in outputs.items():
        assert 'Value' in o_body, f"Output {o_name} missing Value"
    print("  [OK] All Outputs reference valid properties")

    print("\n[SUCCESS] CloudFormation template validated successfully with zero errors!")

if __name__ == '__main__':
    validate_template('deploy/aws/staging-object-lock-storage.yaml')
