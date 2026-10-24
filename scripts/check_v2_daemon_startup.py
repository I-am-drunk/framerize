"""Check explicit V2 startup using a fresh, isolated, account-free daemon."""
import argparse
import http.client
import json
import os
from pathlib import Path
import signal
import subprocess
import time


def request(method, path, body=None):
    connection = http.client.HTTPConnection('127.0.0.1', 18787, timeout=1)
    try:
        payload = None if body is None else json.dumps(body).encode()
        connection.request(method, path, payload, {'Content-Type': 'application/json'})
        response = connection.getresponse()
        data = response.read(16385)
        if len(data) > 16384:
            raise RuntimeError('oversized local response')
        return response.status, data
    finally:
        connection.close()


def isolated():
    for kind in ('net', 'pid'):
        current = os.readlink('/proc/self/ns/' + kind)
        if current == os.environ['CHECK_PARENT_' + kind.upper() + 'NS']:
            raise RuntimeError('private namespace required')
    for path in ('/home/tjninja/.framerize', '/run/user/1000/bus'):
        if Path(path).exists():
            raise RuntimeError('host runtime visible')


def ready(child):
    deadline = time.monotonic() + 8
    while time.monotonic() < deadline:
        if child.poll() is not None:
            raise RuntimeError('daemon exited before readiness')
        try:
            if request('GET', '/api/health') == (200, b'ok'):
                return
        except (OSError, http.client.HTTPException):
            pass
        time.sleep(.05)
    raise RuntimeError('daemon readiness deadline')


def invalid_request():
    return dict(account_id='00000000-0000-4000-8000-000000000001',
        user_id='unused', team_id='unused', project_id='unused',
        conversation_id='unused', page_url='invalid:', route='/ai/v3/responses/',
        feature_category='unused', feature_name='unused', step='unused',
        turn_id='unused', user_prompt='unused', timeout_ms=0, token_limit=1,
        request={})


def child_processes(pid):
    found = set()
    for task in Path('/proc', str(pid), 'task').iterdir():
        found.update((task/'children').read_text().split())
    return sorted(found)


def check(package, config, output):
    isolated()
    output.mkdir(mode=0o700)
    web = output/'web'
    web.mkdir()
    argv = [str(package/'bin/framerize'), 'daemon', '--listen', '127.0.0.1:18787',
            '--engine', 'camoufox', '--db', str(output/'state.db'),
            '--profiles', str(output/'profiles'), '--web-dir', str(web),
            '--sidecar-script', str(package/'host/camoufox_host.py'),
            '--active-pool-size', '1', '--eligible-accounts', 'none',
            '--offline-worker-config', str(config), '--offline-sdk', 'v2']
    report = {'accepted': False, 'argv': argv}
    with (output/'daemon.log').open('xb') as log:
        child = subprocess.Popen(argv, stdin=subprocess.DEVNULL,
                                 stdout=log, stderr=subprocess.STDOUT)
        report['daemon_pid_in_namespace'] = child.pid
        try:
            ready(child)
            status, body = request('GET', '/api/accounts')
            if status != 200 or json.loads(body) != []:
                raise RuntimeError('fresh instance is not empty')
            report['empty_accounts'] = True
            status, body = request('POST', '/api/offline/complete', invalid_request())
            if status != 400 or json.loads(body) != {'error': {'code': 'InvalidRequest'}}:
                raise RuntimeError('explicit offline handler not reached')
            report['offline_input_rejected'] = True
            if request('GET', '/api/hello') != (200, b'ok'):
                raise RuntimeError('normal hello route unavailable')
            report['hello_ready'] = True
            report['children_before_stop'] = child_processes(child.pid)
            if report['children_before_stop']:
                raise RuntimeError('unexpected child in empty instance')
            report['accepted'] = True
        finally:
            if child.poll() is None:
                child.send_signal(signal.SIGTERM)
            try:
                report['daemon_exit'] = child.wait(timeout=3)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=2)
                report['accepted'] = False
                report['forced_kill'] = True
            report['daemon_absent'] = not Path('/proc', str(child.pid)).exists()
            report['accepted'] &= report['daemon_absent']
            (output/'result.json').write_text(json.dumps(report, indent=2)+'\n')
    if not report['accepted']:
        raise RuntimeError('empty daemon check failed')
    return report


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('package', type=Path)
    parser.add_argument('config', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    result = check(args.package, args.config, args.output)
    print(json.dumps({k: v for k, v in result.items() if k != 'argv'}))
