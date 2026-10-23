"""Check normal-route native dispatch in an empty, network-isolated instance."""
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
import urllib.error
import urllib.request

root, binary, host, worker, routing, python = map(Path, sys.argv[1:])
assert root.is_absolute() and root.is_dir() and not any(root.iterdir())
assert os.readlink('/proc/self/ns/net') != os.environ['FRAMERIZE_PARENT_NETNS']
env = {key: os.environ[key] for key in ('PATH', 'HOME', 'LANG') if key in os.environ}
env.update(FRAMERIZE_PYTHON=str(python), FRAMERIZE_GATE_STATE=str(root / 'gate.json'),
           FRAMERIZE_DEVICE_POOL='1', FRAMERIZE_RESERVE_PER_DAY='0',
           TOKIO_WORKER_THREADS='2', PYTHONDONTWRITEBYTECODE='1')
argv = [str(binary), 'daemon', '--listen', '127.0.0.1:18788',
        '--db', str(root / 'accounts.db'), '--profiles', str(root / 'profiles'),
        '--web-dir', str(root / 'web'), '--engine', 'camoufox',
        '--sidecar-script', str(host), '--active-pool-size', '1',
        '--eligible-accounts', 'none', '--offline-worker-config', str(worker),
        '--offline-routing-config', str(routing)]
opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
result = {'accepted': False, 'network': 'private namespace'}
process = None

def request(path, payload=None):
    data = None if payload is None else json.dumps(payload).encode()
    headers = {'Content-Type': 'application/json', 'x-framerize-session': 'binding-fixture'}
    if payload is not None:
        # Synthetic context is valid only in this empty-store network fixture.
        headers['x-framerize-context'] = json.dumps({
            'page_url': 'https://framer.com/projects/binding-fixture',
            'attempts': [{'feature_category': 'fixture', 'feature_name': 'fixture',
                'step': 'fixture', 'turn_id': 'fixture', 'user_prompt': 'fixture only'}]})
    req = urllib.request.Request('http://127.0.0.1:18788' + path, data=data,
                                 headers=headers)
    try:
        response = opener.open(req, timeout=2)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        body = response.read(65537)
        assert len(body) <= 65536, 'response exceeds bound'
        return response.status, body

try:
    with (root / 'server.log').open('xb') as log:
        process = subprocess.Popen(argv, env=env, stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL, stderr=log, start_new_session=True)
    deadline = time.monotonic() + 10
    while True:
        assert process.poll() is None, 'candidate exited during configuration'
        try:
            if request('/api/health') == (200, b'ok'):
                break
        except urllib.error.URLError:
            pass
        assert time.monotonic() < deadline, 'candidate startup expired'
        time.sleep(0.05)
    status, body = request('/api/accounts')
    assert status == 200 and json.loads(body) == [], 'fixture account store is not empty'
    status, body = request('/openai/v1/responses',
        {'model': 'gpt-4.1', 'input': 'fixture only', 'stream': True, 'max_output_tokens': 16})
    error = json.loads(body)['error']
    allowed = {'Ineligible', 'InvalidRequest', 'MissingContext', 'UnsupportedStreaming',
               'InvalidScope', 'PaidDisabled', 'Internal', 'ScopeClosed', 'Overloaded'}
    result['observed_response'] = {'status': status,
        'type': 'native_dispatch_error' if error.get('type') == 'native_dispatch_error' else 'other',
        'code': error.get('code') if error.get('code') in allowed else 'other'}
    assert status == 503, 'unexpected empty-account response status'
    assert error['type'] == 'native_dispatch_error', 'request did not use native dispatch'
    assert error['code'] == 'Ineligible', 'streaming configuration was not accepted'
    assert process.poll() is None, 'native refusal terminated the instance'
    children = set()
    for path in Path('/proc', str(process.pid), 'task').glob('*/children'):
        children.update(path.read_text().split())
    assert not children, 'empty-account refusal started a child process'
    result.update(accepted=True, checks=['normal streaming route uses native dispatch',
        'empty account refusal precedes host and worker launch'],
        http_status=status, error_type=error['type'], error_code=error['code'],
        account_count=0, child_count=0, inference_attempts=0)
    group = Path('/sys/fs/cgroup' + Path('/proc/self/cgroup').read_text().strip().split('::', 1)[1])
    result['resources'] = {name: int((group / name).read_text()) for name in
        ('memory.current', 'memory.peak', 'memory.swap.current', 'pids.current', 'pids.peak')}
    result['memory_events'] = dict(line.split() for line in (group / 'memory.events').read_text().splitlines())
    assert result['resources']['memory.swap.current'] == 0, 'fixture used swap'
    assert all(int(value) == 0 for value in result['memory_events'].values()), 'fixture memory event'
except Exception as error:
    result.update(accepted=False, error_type=type(error).__name__)
    if isinstance(error, AssertionError):
        result['reason'] = str(error)
finally:
    if process is not None:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            process.wait(timeout=5)
            result['cleanup_confirmed'] = True
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=5)
            result.update(accepted=False, cleanup_confirmed=False)
    else:
        result['cleanup_confirmed'] = True
    result['inputs'] = {}
    for name, path in [('binary', binary), ('host', host), ('worker', worker), ('routing', routing)]:
        with path.open('rb') as stream:
            result['inputs'][name] = hashlib.file_digest(stream, 'sha256').hexdigest()
    with (root / 'result.json').open('x') as stream:
        stream.write(json.dumps(result, indent=2) + '\n')
print(json.dumps(result, sort_keys=True))
raise SystemExit(0 if result['accepted'] else 1)
