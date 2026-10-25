"""Start the qualified V3 endpoint with an empty store and no upstream work."""
import http.client
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time


def need(value, reason):
    if not value:
        raise RuntimeError(reason)


def request(method, path, body=None):
    client = http.client.HTTPConnection('127.0.0.1', 18787, timeout=1)
    try:
        client.request(method, path, None if body is None else json.dumps(body),
                       {'Content-Type': 'application/json'})
        reply = client.getresponse()
        data = reply.read(16385)
        need(len(data) <= 16384, 'oversized local response')
        return reply.status, data
    finally:
        client.close()


def children(pid):
    found = set()
    for task in Path('/proc', str(pid), 'task').iterdir():
        found.update((task / 'children').read_text().split())
    return sorted(found)


def ready(child):
    end = time.monotonic() + 8
    while time.monotonic() < end:
        need(child.poll() is None, 'daemon exited before readiness')
        try:
            if request('GET', '/api/health') == (200, b'ok'):
                return
        except (OSError, http.client.HTTPException):
            pass
        time.sleep(.05)
    raise RuntimeError('daemon readiness deadline')


def local_checks():
    checks = []
    for path in ('/api/hello', '/anthropic/api/hello', '/openai/api/hello'):
        for method in ('GET', 'HEAD'):
            need(request(method, path) == (200, b'ok' if method == 'GET' else b''),
                 'normal hello route changed')
            checks.append(method + ' ' + path)
    status, raw = request('GET', '/api/accounts')
    need(status == 200 and json.loads(raw) == [], 'account store not empty')
    checks.append('empty accounts')
    status, raw = request('GET', '/api/pool')
    pool = json.loads(raw)
    need(status == 200 and pool['accounts'] == 0 and
         pool['residency']['residents'] == [], 'unexpected resident')
    checks.append('empty pool')
    status, raw = request('POST', '/api/offline/complete', {})
    need(status == 422 and b'missing field' in raw, 'offline route not installed')
    checks.append('offline input refusal')
    return checks


def check(package, config, output):
    for kind in ('net', 'pid'):
        need(os.readlink('/proc/self/ns/' + kind) !=
             os.environ['CHECK_PARENT_' + kind.upper() + 'NS'], 'namespace shared')
    for path in ('/home/tjninja/.framerize', '/run/user/1000/bus'):
        need(not Path(path).exists(), 'production runtime visible')
    output.mkdir(mode=0o700)
    argv = [str(package / 'bin/framerize'), 'daemon', '--listen', '127.0.0.1:18787',
            '--engine', 'camoufox', '--db', str(output / 'state.db'),
            '--profiles', str(output / 'profiles'), '--web-dir', str(output / 'no-web'),
            '--sidecar-script', str(package / 'host/camoufox_host.py'),
            '--active-pool-size', '1', '--eligible-accounts', 'all',
            '--offline-worker-config', str(config), '--offline-sdk', 'v3']
    report = dict(accepted=False, argv=argv, sdk_calls=0, upstream_requests=0)
    with (output / 'daemon.log').open('xb') as log:
        child = subprocess.Popen(argv, stdin=subprocess.DEVNULL,
                                 stdout=log, stderr=subprocess.STDOUT)
        report['daemon_pid_in_namespace'] = child.pid
        try:
            ready(child)
            need(not children(child.pid), 'unexpected startup child')
            report['checks'] = local_checks()
            report['children_after_checks'] = children(child.pid)
            need(not report['children_after_checks'], 'unexpected child after checks')
            report['accepted'] = True
        finally:
            finish(child, output, report)
    need(report['accepted'], 'startup check failed')
    return report


def finish(child, output, report):
    if child.poll() is None:
        child.send_signal(signal.SIGTERM)
    try:
        report['daemon_exit'] = child.wait(timeout=3)
    except subprocess.TimeoutExpired:
        child.kill()
        child.wait(timeout=2)
        report.update(accepted=False, forced_kill=True)
    report['daemon_absent'] = not Path('/proc', str(child.pid)).exists()
    report['accepted'] &= report['daemon_absent']
    with (output / 'result.json').open('x') as out:
        json.dump(report, out)
        out.write('\n')


if __name__ == '__main__':
    need(len(sys.argv) == 4, 'package/config/output required')
    result = check(*(Path(arg) for arg in sys.argv[1:]))
    print(json.dumps({k: v for k, v in result.items() if k != 'argv'}))
