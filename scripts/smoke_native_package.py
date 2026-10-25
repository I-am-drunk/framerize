"""Check a native package with synthetic state, no SDK and no upstream requests."""
import argparse
import json
import os
from pathlib import Path
import signal
import socket
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parent))
from release import sha256, stage, verify

REPO = Path(__file__).resolve().parents[1]
ACCOUNT = str(uuid.UUID(int=1))
HTTP = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def prepare(root):
    profiles = root / 'profiles'
    (profiles / '.sessions').mkdir(parents=True)
    subprocess.run([sys.executable, '-I', '-B',
        str(REPO / 'runtime/tests_native/seed_ledger.py'),
        str(profiles / '.sidecar/camoufox')], check=True, timeout=10,
        capture_output=True, env={'PATH': '/usr/bin:/bin'})
    account = {'id': ACCOUNT, 'alias': 'fixture', 'framer_user_id': 'fixture-user',
        'castle_device_id': 'fixture-device', 'current_team': 'fixture-team',
        'current_project': 'fixture-project', 'state': 'ready', 'credit': None,
        'consecutive_failures': 0, 'cooldown_step': 0}
    with sqlite3.connect(root / 'accounts.db') as database:
        database.execute('CREATE TABLE accounts (id TEXT PRIMARY KEY, json TEXT NOT NULL)')
        database.execute('INSERT INTO accounts VALUES (?, ?)', (ACCOUNT, json.dumps(account)))
    pins = []
    for name in ('inert-node', 'inert-entry.js'):
        path = root / name
        path.write_text('This synthetic worker must never execute.\n')
        pins.append({'path': str(path), 'sha256': sha256(path)})
    (root / 'worker.json').write_text(json.dumps({'node': pins[0]['path'],
        'entry': pins[1]['path'], 'pins': pins}))


def ready(child, url):
    deadline = time.monotonic() + 10
    while True:
        if child.poll() is not None:
            raise RuntimeError('native service exited before readiness')
        try:
            with HTTP.open(url + '/api/health', timeout=1) as reply:
                assert reply.status == 200
                assert reply.headers.get('x-framerize-pid') == str(child.pid)
            return
        except OSError:
            if time.monotonic() >= deadline:
                raise
            time.sleep(0.05)


def lifecycle(release, root, signum):
    with socket.socket() as reserve:
        reserve.bind(('127.0.0.1', 0))
        address = f'127.0.0.1:{reserve.getsockname()[1]}'
    command = [str(release / 'bin/framerize-native'), 'serve',
        '--db', str(root / 'accounts.db'), '--profiles', str(root / 'profiles'),
        '--worker-config', str(root / 'worker.json'), '--accounts', ACCOUNT,
        '--listen', address, '--python', sys.executable]
    env = {'PATH': '/usr/bin:/bin', 'HOME': str(root),
        'FRAMERIZE_LOG_DIR': str(root / 'logs')}
    with (root / f'native-{signum}.log').open('w') as log:
        child = subprocess.Popen(command, cwd=root, env=env, start_new_session=True,
            stdout=log, stderr=log)
        try:
            ready(child, 'http://' + address)
            invalid = urllib.request.Request('http://' + address + '/api/offline/complete',
                data=b'{}', headers={'Content-Type': 'application/json'})
            try:
                HTTP.open(invalid, timeout=2).close()
                raise AssertionError('invalid request was accepted')
            except urllib.error.HTTPError as error:
                assert error.code in (400, 422)
                error.close()
            contender = command.copy()
            contender[contender.index('--listen') + 1] = '127.0.0.1:0'
            result = subprocess.run(contender, cwd=root, env=env, capture_output=True,
                text=True, timeout=10)
            assert result.returncode != 0
            assert 'session store ownership unavailable' in result.stderr, result.stderr
            ready(child, 'http://' + address)
            os.killpg(child.pid, signum)
            assert child.wait(timeout=10) == 0, 'native shutdown was not clean'
        finally:
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()


def main(archive, worker_config=None):
    root = Path(tempfile.mkdtemp(prefix='framerize-native-package-'))
    release = stage(archive, sha256(archive), root / 'releases')
    info = verify(release)
    assert info['application'] == 'framerize-native'
    assert info['capability'] == 'experimental-native-serving'
    assert not info['source_dirty']
    assert not (release / 'runtime/framerize_host').exists()
    prepare(root)
    config_sha256 = None
    if worker_config is not None:
        with worker_config.open('rb') as stream:
            raw = stream.read(65537)
        if len(raw) > 65536:
            raise ValueError('worker configuration exceeds limit')
        (root / 'worker.json').write_bytes(raw)
        config_sha256 = sha256(root / 'worker.json')
    for signum in (signal.SIGTERM, signal.SIGINT):
        lifecycle(release, root, signum)
    verify(release)
    print(json.dumps({'result': 'pass', 'commit': info['commit'], 'root': str(root),
        'worker_config_sha256': config_sha256,
        'checks': ['archive', 'runtime', 'startup', 'invalid-request',
                   'profile-exclusion', 'SIGTERM', 'SIGINT', 'profile-reuse']}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('archive', type=Path)
    parser.add_argument('--worker-config', type=Path,
        help='verify supplied worker files during startup; never send a valid SDK request')
    args = parser.parse_args()
    main(args.archive.resolve(), args.worker_config)
