"""Bounded integration fixture for a separately built browserless executable."""
import hashlib
import json
import os
from pathlib import Path
import selectors
import signal
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from prepare_browser_runtime import prepare

root, binary, python, config = map(Path, sys.argv[1:])
assert root.is_absolute() and root.is_dir() and not any(root.iterdir())
assert os.readlink('/proc/self/ns/net') != os.environ['FRAMERIZE_PARENT_NETNS'], 'private network required'
processes = []
checks = []

def start(argv, name, **kwargs):
    log = (root / (name + '.log')).open('wb')
    child = subprocess.Popen(argv, stderr=log, start_new_session=True, **kwargs)
    log.close()
    processes.append(child)
    return child

def stop(child):
    try:
        os.killpg(child.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        child.wait(timeout=5)
        processes.remove(child)
    except subprocess.TimeoutExpired:
        os.killpg(child.pid, signal.SIGKILL)
        child.wait(timeout=5)
        raise AssertionError('fixture cleanup exceeded deadline')

def run():
    profiles = root / 'profiles'
    (profiles / '.sidecar/camoufox').mkdir(parents=True)
    code = root / 'runtime'
    package = prepare(code)
    poison = code / 'cooldown-handoff'
    poison.mkdir(mode=0o700)
    pending = poison / 'pending.json'
    pending.write_text('{stale fixture, never load from the code directory')
    pending.chmod(0o600)
    env = {key: os.environ[key] for key in ('PATH', 'HOME', 'LANG') if key in os.environ}
    env.update(FRAMERIZE_STATE=str(profiles / '.sidecar/camoufox'),
        FRAMERIZE_LOG_DIR=str(profiles / '.logs'), FRAMERIZE_DEVICE_POOL='1',
        FRAMERIZE_RESERVE_PER_DAY='0', FRAMERIZE_GATE_STATE=str(root / 'gate-state.json'),
        FRAMERIZE_PYTHON=str(python), TOKIO_WORKER_THREADS='2', PYTHONDONTWRITEBYTECODE='1')
    host = start([str(python), '-B', str(code / 'camoufox_host.py')], 'host',
        env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE)
    command = dict(id=1, cmd='browser_status', account='00000000-0000-0000-0000-000000000001')
    host.stdin.write((json.dumps(command) + '\n').encode())
    host.stdin.flush()
    with selectors.DefaultSelector() as ready:
        ready.register(host.stdout, selectors.EVENT_READ)
        assert ready.select(timeout=12), 'host did not answer within its startup bound'
        line = host.stdout.readline(65537)
    assert 0 < len(line) <= 65536, 'invalid host response size'
    reply = json.loads(line)
    assert reply['id'] == 1 and reply['ok'] is True, 'host did not start with isolated state'
    assert reply['result']['state'] == 'absent', 'fixture started a browser'
    assert pending.read_text().startswith('{stale fixture'), 'shared fixture was changed'
    checks.append('host ignores code-directory handoff and starts with isolated state')
    stop(host)
    with socket.socket() as probe:
        probe.bind(('127.0.0.1', 0))
        port = probe.getsockname()[1]
    argv = [str(binary), 'daemon', '--listen', f'127.0.0.1:{port}',
        '--db', str(root / 'framerize.db'), '--profiles', str(profiles),
        '--web-dir', str(root / 'web'), '--engine', 'camoufox',
        '--sidecar-script', str(code / 'camoufox_host.py'), '--active-pool-size', '1',
        '--eligible-accounts', 'none', '--offline-worker-config', str(config)]
    daemon = start(argv, 'daemon', env=env, stdout=subprocess.DEVNULL)
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    def request(path, data=None, method=None):
        req = urllib.request.Request(f'http://127.0.0.1:{port}' + path, data=data,
            method=method, headers={'Content-Type': 'application/json'})
        try:
            response = opener.open(req, timeout=2)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            return response.status, response.read(65537)
    deadline = time.monotonic() + 10
    while True:
        assert daemon.poll() is None, 'isolated daemon exited during startup'
        try:
            if request('/api/health')[0] == 200:
                break
        except urllib.error.URLError:
            pass
        assert time.monotonic() < deadline, 'isolated daemon startup expired'
        time.sleep(0.05)
    checks.append('separate daemon serves health with empty isolated account store')
    assert request('/anthropic/api/hello', method='HEAD') == (200, b'')
    assert request('/api/offline/complete', b'{')[0] == 400
    assert request('/api/offline/complete', b'{}')[0] == 422
    payload = dict(account_id=command['account'], user_id='fixture-user',
        team_id='fixture-team', project_id='fixture-project', conversation_id='fixture-conversation',
        page_url='https://framer.com/projects/fixture-project', route='/ai/v3/responses/',
        feature_category='fixture-category', feature_name='fixture-feature', step='fixture-step',
        turn_id='fixture-turn', user_prompt='fixture annotation', timeout_ms=3000, token_limit=32,
        request=dict(model='gpt-4.1', input='fixture text', stream=False))
    status, body = request('/api/offline/complete', json.dumps(payload).encode())
    assert status == 503 and json.loads(body)['error']['code'] == 'Ineligible'
    assert request('/api/health')[0] == 200 and daemon.poll() is None
    checks.append('malformed JSON and unknown account are refused without crashing')
    stop(daemon)
    return dict(accepted=True, checks=checks, network='isolated namespace',
        accounts='synthetic request; empty store; no session credentials', package=package,
        binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest())


result = dict(accepted=False)
try:
    result = run()
except Exception as error:
    result.update(error_type=type(error).__name__, checks=checks)
    if isinstance(error, AssertionError):
        result['reason'] = str(error)
finally:
    for child in tuple(processes):
        try:
            stop(child)
        except Exception:
            result.update(accepted=False, cleanup_confirmed=False)
    result.setdefault('cleanup_confirmed', not processes)
    (root / 'result.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps(result, sort_keys=True))
raise SystemExit(0 if result['accepted'] else 1)
