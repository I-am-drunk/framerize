"""Return an owned canary transition to a compatible browser service."""
import fcntl, hashlib, json, os, pathlib, stat, subprocess, sys, time, urllib.request

HERE = pathlib.Path(__file__).resolve().parent
UNIT = 'framerize.service'
CGROUP_ROOT = pathlib.Path('/sys/fs/cgroup')


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


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


def ready(expected, seconds=60):
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
                with http.open('http://127.0.0.1:8787/api/health', timeout=1) as reply:
                    if reply.status == 200 and reply.headers.get('x-framerize-pid') == str(pid):
                        return pid
            except OSError: pass
        time.sleep(0.2)
    raise RuntimeError('browser return did not become ready')


def has_history(plan):
    path = pathlib.Path(plan['profiles']) / '.sidecar/camoufox/rate-history.sqlite3'
    try:
        mode = path.lstat().st_mode
    except FileNotFoundError:
        return False
    require(stat.S_ISREG(mode), 'history is not a regular file')
    return True


def stopped(plan):
    current = status()
    require(current['MainPID'] == '0' and current['ActiveState'] in ('inactive', 'failed'),
            'service shutdown is unconfirmed')
    group = plan['expected']['service']['ControlGroup']
    require(group.startswith('/user.slice/') and '..' not in group.split('/'),
            'unexpected service cgroup')
    events = CGROUP_ROOT / group.lstrip('/') / 'cgroup.events'
    if events.exists():
        values = dict(line.split() for line in events.read_text().splitlines())
        require(values.get('populated') == '0', 'service descendants remain')


def finish_return(kind, pid):
    saved = json.loads((HERE / 'pool-config.json').read_text())
    observed = http_json('/api/pool/config')
    fenced = dict(saved, eligible_accounts=[])
    require(observed in (saved, fenced), 'pool configuration changed outside this transition')
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
    pid = ready(target)
    return finish_return(kind, pid)


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
    if dropin.exists() or fallback.exists() or state['phase'].startswith('restoring-'):
        return restore_started(plan, dropin)
    require(state['phase'] in ('armed', 'fencing', 'fenced', 'restored-original',
            'restoring-original'), 'transition configuration is missing')
    require(pid and executable(pid) == plan['expected']['binary'], 'original service is missing')
    require(not has_history(plan), 'original service cannot resume migrated history')
    expected_invocation = state.get('invocation', plan['expected']['service']['InvocationID'])
    require(current['InvocationID'] == expected_invocation, 'original service owner changed')
    ready(plan['expected']['binary'])
    return finish_return('original', pid)


def restore():
    flags = os.O_CREAT | os.O_RDWR | os.O_CLOEXEC | os.O_NOFOLLOW
    with os.fdopen(os.open(HERE / 'recovery.lock', flags, 0o600), 'rb') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return restore_locked()


def main(directory):
    global HERE
    HERE = pathlib.Path(directory).resolve(strict=True)
    try:
        result = restore()
    except Exception as error:
        print(json.dumps({'status': 'failed', 'error_type': type(error).__name__}), flush=True)
        return 1
    print(json.dumps(result), flush=True)
    return 0


if __name__ == '__main__':
    if len(sys.argv) != 2:
        raise SystemExit('usage: recovery.py PLAN_DIRECTORY')
    raise SystemExit(main(sys.argv[1]))
