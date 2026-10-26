"""Return an owned canary transition to a compatible browser service."""
import fcntl, hashlib, json, os, pathlib, stat, subprocess, sys, time, urllib.request, uuid

HERE = pathlib.Path(__file__).resolve().parent
UNIT = 'framerize.service'
CGROUP_ROOT = pathlib.Path('/sys/fs/cgroup')


class RecoveryFailure(RuntimeError):
    """A fixed diagnostic owned by this module, safe for the operator log."""
    pass


class ServiceShutdownUnconfirmed(RecoveryFailure):
    pass


class ServiceDescendantsRemain(RecoveryFailure):
    pass


def require(condition, message):
    if not condition:
        raise RecoveryFailure(message)


def digest(path):
    with pathlib.Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def command(*args):
    return subprocess.check_output(['/usr/bin/systemctl', '--user', *args],
        text=True, timeout=420)


def status():
    text = command('show', UNIT, '--property=MainPID,ActiveState,InvocationID')
    return dict(line.split('=', 1) for line in text.splitlines())


def read_plan():
    plan = json.loads((HERE / 'plan.json').read_text())
    require(plan['version'] == 1, 'unsupported transition plan')
    require('pool-config.json' in plan['artifacts'], 'pool configuration is not pinned')
    for name, expected in plan['artifacts'].items():
        require(digest(HERE / name) == expected, 'transition artifact changed')
    for path, expected in plan['binaries'].items():
        require(digest(path) == expected, 'transition executable changed')
    require(digest(plan['expected']['binary']) == plan['expected']['binary_sha256'],
            'original executable changed')
    return plan


def atomic(path, data):
    temporary = path.with_name(path.name + '.new-' + str(os.getpid()))
    descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(descriptor, 'wb') as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        temporary.replace(path)
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try: os.fsync(directory)
        finally: os.close(directory)
    finally:
        temporary.unlink(missing_ok=True)


def executable(pid):
    return str(pathlib.Path('/proc', str(pid), 'exe').resolve(strict=True))


def remember(phase, **facts):
    atomic(HERE / 'state.json', json.dumps(dict(phase=phase, at=time.time(), **facts)).encode())


def http_json(path, value=None):
    data = None if value is None else json.dumps(value).encode()
    request = urllib.request.Request('http://127.0.0.1:8787' + path, data=data,
        headers={'Content-Type': 'application/json'})
    http = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with http.open(request, timeout=65) as response:
        return json.loads(response.read(262145))


def narrower_pool(before, after):
    if not isinstance(before, dict) or not isinstance(after, dict) or 'eligible_accounts' not in before:
        return False
    if dict(after, eligible_accounts=before['eligible_accounts']) != before:
        return False
    selected = after.get('eligible_accounts')
    if not isinstance(selected, list) or not all(isinstance(v, str) for v in selected):
        return False
    try:
        if len(set(selected)) != len(selected) or any(str(uuid.UUID(v)) != v for v in selected):
            return False
    except ValueError:
        return False
    allowed = before['eligible_accounts']
    return allowed == 'all' or isinstance(allowed, list) and set(selected) <= set(allowed)


def pool_change(plan, saved, before, after):
    require(before == saved or narrower_pool(saved, before), 'invalid prior drain configuration')
    require(narrower_pool(before, after), 'drain configuration widens admission')
    return dict(version=1, invocation=plan['expected']['service']['InvocationID'],
                saved_sha256=plan['artifacts']['pool-config.json'], before=before, after=after)


def record_pool_change(plan, saved, before, after):
    record = pool_change(plan, saved, before, after)
    atomic(HERE / 'pool-write.json', json.dumps(record).encode())


def owned_pool_versions(plan, saved):
    versions = [saved, dict(saved, eligible_accounts=[])]
    path = HERE / 'pool-write.json'
    if path.exists():
        record = json.loads(path.read_text())
        require(isinstance(record, dict), 'invalid drain configuration record')
        expected = pool_change(plan, saved, record.get('before'), record.get('after'))
        require(record == expected, 'drain configuration record changed')
        versions.extend((record['before'], record['after']))
    return versions


def ready(expected, seconds=60, *, verify_history=False):
    http = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        current = status()
        pid = int(current['MainPID'])
        try:
            matching = pid and current['ActiveState'] == 'active' and executable(pid) == expected
        except (FileNotFoundError, ProcessLookupError):
            matching = False
        if matching:
            try:
                path = '/api/ready' if verify_history else '/api/health'
                with http.open('http://127.0.0.1:8787' + path, timeout=1) as reply:
                    valid_history = True
                    if verify_history:
                        raw = reply.read(4097)
                        value = json.loads(raw) if len(raw) <= 4096 else None
                        valid_history = type(value) is dict and value.get('ready') is True
                    if (reply.status == 200 and reply.headers.get('x-framerize-pid') == str(pid)
                            and valid_history):
                        return pid
            except (OSError, ValueError, UnicodeError): pass
        time.sleep(0.2)
    raise RecoveryFailure('browser return did not become ready: local account history readiness failed'
                          if verify_history else 'browser return did not become healthy')


def has_history(plan):
    path = pathlib.Path(plan['profiles']) / '.sidecar/camoufox/rate-history.sqlite3'
    try:
        mode = path.lstat().st_mode
    except FileNotFoundError:
        return False
    require(stat.S_ISREG(mode), 'history is not a regular file')
    return True


def stopped(plan):
    deadline = time.monotonic() + 10
    group = plan['expected']['service']['ControlGroup']
    require(group.startswith('/user.slice/') and '..' not in group.split('/'),
            'unexpected service cgroup')
    events = CGROUP_ROOT / group.lstrip('/') / 'cgroup.events'
    while True:
        current = status()
        idle = current['MainPID'] == '0' and current['ActiveState'] in ('inactive', 'failed')
        try:
            values = dict(line.split() for line in events.read_text().splitlines())
            empty = values.get('populated') == '0'
        except FileNotFoundError:
            empty = True
        if idle and empty:
            return
        if time.monotonic() >= deadline:
            if not idle:
                raise ServiceShutdownUnconfirmed('service shutdown is unconfirmed')
            raise ServiceDescendantsRemain('service descendants remain')
        time.sleep(0.1)


def finish_return(kind, pid, plan):
    saved = json.loads((HERE / 'pool-config.json').read_text())
    observed = http_json('/api/pool/config')
    require(observed in owned_pool_versions(plan, saved),
            'pool configuration changed outside this transition')
    if observed != saved:
        http_json('/api/pool/config', saved)
    require(http_json('/api/pool/config') == saved, 'pool configuration was not restored')
    current = status()
    require(current['MainPID'] == str(pid) and current['ActiveState'] == 'active',
            'returned service changed')
    remember('restored-' + kind, pid=pid, invocation=current['InvocationID'])
    return {'status': 'restored-' + kind, 'pid': pid}


def restore_started(plan, dropin):
    current = status()
    pid = int(current['MainPID'])
    running = executable(pid) if pid else None
    if running == plan['native']:
        remember('restoring-stop', invocation=current['InvocationID'])
        command('stop', UNIT)
        stopped(plan)
        running = None
    elif not pid:
        stopped(plan)
    migrated = has_history(plan)
    target = plan['browser'] if migrated else plan['expected']['binary']
    require(running in (None, target), 'a different browser service is still running')
    kind = 'browser' if migrated else 'original'
    remember('restoring-' + kind)
    fallback = pathlib.Path(plan['return_dropin'])
    if migrated:
        atomic(fallback, (HERE / 'browser.conf').read_bytes())
    else:
        fallback.unlink(missing_ok=True)
    dropin.unlink(missing_ok=True)
    command('daemon-reload')
    if running is None:
        command('start', UNIT)
    pid = ready(target, verify_history=migrated)
    return finish_return(kind, pid, plan)


def restore_interrupted_stop(plan, dropin):
    current = status()
    pid = int(current['MainPID'])
    expected = plan['expected']['service']['InvocationID']
    require(current['InvocationID'] == expected or
            not pid and current['ActiveState'] in ('inactive', 'failed') and
            not current['InvocationID'], 'original service owner changed')
    require(not pid or executable(pid) == plan['expected']['binary'],
            'original service executable changed')
    if pid or current['ActiveState'] == 'deactivating':
        command('stop', UNIT)
    stopped(plan)
    return restore_started(plan, dropin)


def restore_locked():
    plan = read_plan()
    if not (HERE / 'state.json').exists(): return {'status': 'not-started'}
    dropin = pathlib.Path(plan['dropin'])
    current = status()
    pid = int(current['MainPID'])
    allowed = {plan['native'], plan['browser'], plan['expected']['binary']}
    require(not pid or executable(pid) in allowed,
            'another service executable owns this unit')
    for name, expected in plan['expected']['service_file_pins'].items():
        require(digest(name) == expected, 'original service configuration changed')
    state = json.loads((HERE / 'state.json').read_text())
    if pid and executable(pid) == plan['expected']['binary']:
        if state['phase'] not in ('restoring-original', 'restored-original'):
            require(current['InvocationID'] == plan['expected']['service']['InvocationID'],
                    'original service owner changed')
    fallback = pathlib.Path(plan['return_dropin'])
    for path, names in [(dropin, ('native.conf',)), (fallback, ('browser.conf',))]:
        require(not path.is_symlink(), 'unexpected transition symlink')
        if path.exists():
            require(path.is_file() and digest(path) in {plan['artifacts'][n] for n in names},
                    'transition configuration changed')
    if state['phase'] in ('stopping-original', 'original-stopped'):
        return restore_interrupted_stop(plan, dropin)
    if dropin.exists() or fallback.exists() or state['phase'].startswith('restoring-'):
        return restore_started(plan, dropin)
    require(state['phase'] in ('armed', 'draining', 'fencing', 'fenced', 'restored-original',
            'restoring-original'), 'transition configuration is missing')
    require(pid and executable(pid) == plan['expected']['binary'], 'original service is missing')
    require(not has_history(plan), 'original service cannot resume migrated history')
    expected_invocation = state.get('invocation', plan['expected']['service']['InvocationID'])
    require(current['InvocationID'] == expected_invocation, 'original service owner changed')
    ready(plan['expected']['binary'])
    return finish_return('original', pid, plan)


def restore():
    flags = os.O_CREAT | os.O_RDWR | os.O_CLOEXEC | os.O_NOFOLLOW
    with os.fdopen(os.open(HERE / 'recovery.lock', flags, 0o600), 'rb') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return restore_locked()


def failure_details(error):
    result = {'error_type': type(error).__name__}
    if isinstance(error, RecoveryFailure):
        result['reason'] = str(error)
    if isinstance(error, subprocess.CalledProcessError):
        result['exit_code'] = error.returncode
    if isinstance(error, OSError):
        result['errno'] = error.errno
    return result


def main(directory):
    global HERE
    HERE = pathlib.Path(directory).resolve(strict=True)
    try:
        result = restore()
    except Exception as error:
        print(json.dumps({'status': 'failed', **failure_details(error)}), flush=True)
        return 1
    print(json.dumps(result), flush=True)
    return 0


if __name__ == '__main__':
    if len(sys.argv) != 2:
        raise SystemExit('usage: recovery.py PLAN_DIRECTORY')
    raise SystemExit(main(sys.argv[1]))
