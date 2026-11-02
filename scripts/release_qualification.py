"""Require recent real inference evidence for the exact browser package."""
import json
import time
from pathlib import Path

from release import sha256, verify


def validate(package, receipt_path):
    if receipt_path is None:
        raise ValueError('isolated OpenAI and Claude inference qualification is required before activation')
    package, receipt_path = Path(package), Path(receipt_path)
    record = json.loads(receipt_path.read_text())
    if type(record) is not dict:
        raise ValueError('qualification must be a JSON object')
    info = verify(package)
    expected = dict(format=1, application='framerize', commit=info.get('commit'),
        binary_sha256=sha256(package / 'bin/framerize'),
        manifest_sha256=sha256(package / 'manifest.json'), engine='camoufox')
    if any(record.get(key) != value for key, value in expected.items()):
        raise ValueError('qualification does not identify this exact browser package')
    timestamp = record.get('verified_at')
    if type(timestamp) not in (int, float) or not 0 <= time.time() - timestamp <= 86400:
        raise ValueError('isolated inference qualification is missing or older than 24 hours')
    isolated = record.get('isolated', {})
    if type(isolated) is not dict:
        raise ValueError('qualification has no isolated runtime record')
    port = isolated.get('port')
    if type(port) is not int or not 8788 <= port <= 65535:
        raise ValueError('qualification must use a separate non-production port')
    for key in ('account_outside_production', 'workspace_outside_production', 'session_lock_owned'):
        if isolated.get(key) is not True:
            raise ValueError('qualification lacks exclusive test-account ownership: ' + key)
    for key in ('shared_token_socket', 'paid_routes_enabled'):
        if isolated.get(key) is not False:
            raise ValueError('qualification used an unsupported runtime setting: ' + key)
    required = {(route, stream) for route in (
        '/openai/v1/responses', '/anthropic/v1/messages') for stream in (False, True)}
    passed = set()
    checks = record.get('checks', [])
    if type(checks) is not list or any(type(check) is not dict for check in checks):
        raise ValueError('qualification checks must be a list of request results')
    for check in checks:
        if (type(check.get('http_status')) is int and check['http_status'] == 200
                and check.get('terminal') is True
                and check.get('expected_content') is True
                and type(check.get('stream')) is bool):
            passed.add((check.get('route'), check['stream']))
    if not required <= passed:
        raise ValueError('isolated streaming and buffered OpenAI/Claude delivery must all pass')
    evidence = record.get('evidence_sha256')
    if type(evidence) is not dict or not evidence:
        raise ValueError('qualification has no recorded verification evidence')
    if 'inference-checks.json' not in evidence:
        raise ValueError('qualification lacks the recorded inference results')
    for name, digest in evidence.items():
        if not name or Path(name).name != name or name in ('.', '..'):
            raise ValueError('qualification evidence must stay beside its receipt')
        path = receipt_path.parent / name
        if path.is_symlink() or not path.is_file() or sha256(path) != digest:
            raise ValueError('qualification evidence changed: ' + name)
    actual_checks = json.loads((receipt_path.parent / 'inference-checks.json').read_text())
    if actual_checks != checks:
        raise ValueError('qualification claims differ from the recorded inference results')
    return record
