"""One bounded canary transition, with an external recovery supervisor required."""
import importlib.util
from contextlib import closing
import json
import os
from pathlib import Path
import sqlite3
import sys
import time
import urllib.error
import urllib.request

spec = importlib.util.spec_from_file_location('recovery', Path(__file__).with_name('recovery.py'))
r = importlib.util.module_from_spec(spec)
spec.loader.exec_module(r)
NATIVE = 'http://127.0.0.1:18791'


def report(phase, **facts):
    r.remember(phase, **facts)
    print(json.dumps(dict(phase=phase, **facts)), flush=True)


def supervised(plan):
    fields = 'MainPID,InvocationID,ExecStopPost,RuntimeMaxUSec,TimeoutStopUSec'
    text = r.command('show', plan['supervisor_unit'], '--property=' + fields)
    info = dict(line.split('=', 1) for line in text.splitlines())
    r.require(info['MainPID'] == str(os.getpid())
              and info['InvocationID'] == os.environ.get('INVOCATION_ID'), 'recovery supervisor differs')
    expected = '/usr/bin/python3 -I -B ' + str(Path(__file__).with_name('recovery.py')) + ' ' + str(r.HERE)
    r.require('argv[]=' + expected + ' ;' in info['ExecStopPost']
              and 'ignore_errors=no' in info['ExecStopPost'], 'recovery is not armed')
    r.require(info['RuntimeMaxUSec'] == '20min' and info['TimeoutStopUSec'] == '10min',
              'supervisor time bounds differ')


def preflight(plan):
    r.require(not (r.HERE / 'state.json').exists(), 'transition has already been used')
    r.require(not (r.HERE / 'pool-write.json').exists(), 'transition has an existing pool write')
    r.require(not r.has_history(plan), 'this transition requires unmigrated history')
    for name in ('dropin', 'return_dropin'):
        path = Path(plan[name])
        r.require(not path.exists() and not path.is_symlink(), 'transition override already exists')
    expected = plan['expected']
    r.require(Path('/proc/sys/kernel/random/boot_id').read_text().strip() == expected['boot_id'],
              'boot changed since preparation')
    current = r.status()
    r.require(all(current[k] == expected['service'][k] for k in
              ('MainPID', 'ActiveState', 'InvocationID')), 'original service changed')
    r.require(r.executable(int(current['MainPID'])) == expected['binary'], 'original binary changed')
    for path, digest in expected['service_file_pins'].items():
        r.require(r.digest(path) == digest, 'original service configuration changed')
    saved = json.loads((r.HERE / 'pool-config.json').read_text())
    r.require(r.http_json('/api/pool/config') == saved, 'pool changed since preparation')
    return saved


def original_owner(plan):
    current = r.status()
    expected = plan['expected']['service']
    r.require(all(current[k] == expected[k] for k in
              ('MainPID', 'ActiveState', 'InvocationID')), 'original service changed')


def drain_step(plan, saved, current, deadline):
    original_owner(plan)
    r.require(r.http_json('/api/pool/config') == current, 'pool changed during drain')
    slots = r.http_json('/api/pool')['residency']['residents']
    r.require(time.monotonic() < deadline, 'active work did not drain within the bound')
    if any(slot['close_started'] for slot in slots):
        return current, False
    active = sorted({slot['account'] for slot in slots if slot['users'] > 0})
    narrowed = dict(current, eligible_accounts=active)
    if narrowed != current:
        report('draining', remaining_accounts=len(active))
        original_owner(plan)
        r.record_pool_change(plan, saved, current, narrowed)
        r.require(r.http_json('/api/pool/config', narrowed) == narrowed,
                  'pool drain was not installed')
        current = narrowed
    return current, not active


def fence(plan, saved, seconds=180):
    deadline = time.monotonic() + seconds
    current = saved
    while True:
        current, empty = drain_step(plan, saved, current, deadline)
        if empty:
            break
        time.sleep(1)
    fenced = dict(saved, eligible_accounts=[])
    snapshot = r.http_json('/api/pool')['residency']
    r.require(snapshot['occupied'] == 0 and snapshot['residents'] == [],
              'resident cleanup is unconfirmed')
    r.require(r.http_json('/api/pool/config') == fenced, 'pool fence changed')
    report('fenced')


def native_http(path, data=None, timeout=1):
    request = urllib.request.Request(NATIVE + path, data=data,
        headers={'Content-Type': 'application/json'})
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    try:
        reply = opener.open(request, timeout=timeout)
    except urllib.error.HTTPError as error:
        reply = error
    with reply:
        body = reply.read(1_048_577)
        r.require(len(body) <= 1_048_576, 'canary reply exceeds bound')
        return reply.status, reply.headers, body


def native_ready(plan, seconds=720):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        current = r.status()
        pid = int(current['MainPID'])
        r.require(pid and current['ActiveState'] in ('active', 'activating'),
                  'native startup exited')
        r.require(r.executable(pid) == plan['native'], 'native service owner changed')
        try:
            status, headers, _ = native_http('/api/health')
            if status == 200 and headers.get('x-framerize-pid') == str(pid):
                return current
        except OSError:
            pass
        time.sleep(1)
    raise r.RecoveryFailure('native readiness deadline expired')


def verify_history(plan, launched_at, current):
    path = Path(plan['profiles']) / '.sidecar/camoufox/rate-history.sqlite3'
    with closing(sqlite3.connect(path.as_uri() + '?mode=ro', uri=True)) as db:
        rows = db.execute('SELECT record FROM migration_evidence').fetchall()
        count = db.execute('SELECT COUNT(*) FROM accounts').fetchone()[0]
    r.require(len(rows) == 1 and count > 0, 'missing migration evidence')
    evidence = json.loads(rows[0][0])
    source = json.loads((r.HERE / 'source-runtime.json').read_text())
    successor = dict(source, invocation=current['InvocationID'])
    r.require(evidence['kind'] == 'legacy-expiry' and evidence['source_runtime'] == source
              and evidence['successor_runtime'] == successor
              and successor['invocation'] != source['invocation'], 'migration owner differs')
    start, end, window = (evidence[k] for k in
                          ('started_at', 'completed_at', 'window_seconds'))
    r.require(window >= 600 and launched_at <= start <= end <= time.monotonic()
              and end - start >= window, 'migration did not observe the required interval')
    r.require(evidence['reserve_blocked_until'] >= start + 86400,
              'unknown legacy reserves were not retained')
    return {'accounts': count, 'observed_seconds': end - start}


def claim_attempt():
    path = r.HERE / 'canary-attempt.json'
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'w') as stream:
        json.dump({'claimed_at': time.time()}, stream)
        stream.flush()
        os.fsync(stream.fileno())
    fd = os.open(r.HERE, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def response_text(value):
    output = value.get('output')
    if not isinstance(output, list):
        return ''
    parts = []
    for item in output:
        if not isinstance(item, dict) or not isinstance(item.get('content'), list):
            continue
        for part in item['content']:
            if isinstance(part, dict) and part.get('type') == 'output_text':
                if not isinstance(part.get('text'), str):
                    return ''
                parts.append(part['text'])
    return ''.join(parts)


def send_once(plan, current):
    r.require(r.status() == current, 'native service changed before canary')
    path = r.HERE / 'request.json'
    r.require(r.digest(path) == plan['artifacts']['request.json'], 'canary request changed')
    claim_attempt()
    report('canary-sending')
    started = time.monotonic()
    status, _, body = native_http('/api/offline/complete', path.read_bytes(), timeout=75)
    r.atomic(r.HERE / 'private-response.json', body)
    try:
        value = json.loads(body)
    except (ValueError, UnicodeError):
        value = None
    complete = status == 200 and isinstance(value, dict) and value.get('status') == 'completed'
    text = response_text(value) if complete else ''
    result = dict(http_status=status, complete=complete, text_is_ok=text.strip() == 'OK',
                  duration_s=time.monotonic() - started, response_bytes=len(body))
    r.atomic(r.HERE / 'canary-result.json', json.dumps(result).encode())
    return result


def switch(plan, saved):
    original_owner(plan)
    r.require(r.http_json('/api/pool/config') == dict(saved, eligible_accounts=[]), 'pool fence changed')
    slots = r.http_json('/api/pool')['residency']
    r.require(slots['occupied'] == 0 and slots['residents'] == [], 'pool is not drained')
    report('stopping-original')
    original_owner(plan)
    r.command('stop', r.UNIT)
    r.stopped(plan)
    report('original-stopped')
    for key, name in [('return_dropin', 'browser.conf'), ('dropin', 'native.conf')]:
        path = Path(plan[key])
        path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        r.atomic(path, (r.HERE / name).read_bytes())
    report('switching')
    r.command('daemon-reload')
    r.stopped(plan)
    launched_at = time.monotonic()
    r.command('start', r.UNIT)
    report('native-started')
    return launched_at


def execute():
    plan = r.read_plan()
    supervised(plan)
    saved = preflight(plan)
    report('armed')
    fence(plan, saved)
    launched_at = switch(plan, saved)
    current = native_ready(plan)
    history = verify_history(plan, launched_at, current)
    report('native-ready', **history)
    result = send_once(plan, current)
    print(json.dumps(dict(phase='canary-result', **result)), flush=True)
    return 0 if result['complete'] and result['text_is_ok'] else 1


def main(directory):
    r.HERE = Path(directory).resolve(strict=True)
    code = 1
    try:
        code = execute()
    except Exception as error:
        print(json.dumps({'phase': 'transition-failed', **r.failure_details(error)}), flush=True)
    try:
        print(json.dumps(r.restore()), flush=True)
    except Exception as error:
        print(json.dumps({'phase': 'recovery-pending', **r.failure_details(error)}), flush=True)
        code = 2
    return code


if __name__ == '__main__':
    if len(sys.argv) != 2:
        raise SystemExit('usage: transition.py PLAN_DIRECTORY')
    raise SystemExit(main(sys.argv[1]))
