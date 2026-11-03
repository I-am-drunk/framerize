"""Check a browser archive with empty account state and no browser or SDK work."""
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
import urllib.request

sys.path.insert(0, str(Path(__file__).resolve().parent))
from release import sha256, stage, verify

HTTP = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def ready(child, url):
    deadline = time.monotonic() + 10
    while True:
        if child.poll() is not None:
            raise RuntimeError('browser service exited before readiness')
        try:
            with HTTP.open(url + '/api/health', timeout=1) as reply:
                assert reply.status == 200
                assert reply.headers.get('x-framerize-pid') == str(child.pid)
            return
        except OSError:
            if time.monotonic() >= deadline:
                raise
            time.sleep(0.05)


def lifecycle(release, root, env, signum):
    with socket.socket() as reserve:
        reserve.bind(('127.0.0.1', 0))
        address = f'127.0.0.1:{reserve.getsockname()[1]}'
    command = [str(release / 'bin/framerize'), 'daemon', '--listen', address,
        '--db', str(root / 'accounts.db'), '--profiles', str(root / 'profiles'),
        '--engine', 'camoufox', '--eligible-accounts', 'none']
    with (root / f'browser-{signum}.log').open('w') as log:
        child = subprocess.Popen(command, cwd=root, env=env, start_new_session=True,
            stdout=log, stderr=log)
        try:
            ready(child, 'http://' + address)
            with HTTP.open('http://' + address + '/', timeout=2) as reply:
                assert reply.read() == (release / 'web/index.html').read_bytes()
            contender = command.copy()
            contender[contender.index('--listen') + 1] = '127.0.0.1:0'
            result = subprocess.run(contender, cwd=root, env=env, capture_output=True,
                text=True, timeout=10)
            assert result.returncode != 0
            assert 'session store ownership unavailable' in result.stderr, result.stderr
            ready(child, 'http://' + address)
            os.killpg(child.pid, signum)
            assert child.wait(timeout=10) == 0, 'browser service shutdown was not clean'
        finally:
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()


def main(archive):
    root = Path(tempfile.mkdtemp(prefix='framerize-browser-package-'))
    release = stage(archive, sha256(archive), root / 'releases')
    info = verify(release)
    assert info['application'] == 'framerize'
    assert info['capability'] == 'browser-serving' and not info['source_dirty']
    assert info['rate_history_schema'] == 2 and info['activation_protocol'] == 1
    from release_checkpoint_history import formats
    readers, writer = formats(info)
    assert readers == {1, 2} and writer in (1, 2)
    actual = json.loads(subprocess.check_output(
        [str(release / 'bin/framerize'), 'checkpoint-history'], text=True, timeout=10))
    assert actual == info['checkpoint_history'], 'binary checkpoint capabilities differ from package'
    assert not (release / 'runtime/framerize_native').exists()
    assert (release / 'share/systemd/user/framerize.service').is_file()
    env = {'PATH': os.defpath, 'HOME': str(root), 'FRAMERIZE_PYTHON': sys.executable,
        'FRAMERIZE_STATE': str(root / 'host-state'), 'FRAMERIZE_LOG_DIR': str(root / 'logs'),
        'FRAMERIZE_GATE_STATE': str(root / 'gate.json'), 'PYTHONDONTWRITEBYTECODE': '1'}
    host = subprocess.run([sys.executable, '-B', str(release / 'runtime/host.py')],
        input='{"cmd":"shutdown"}\n', text=True, capture_output=True,
        cwd=root, env=env, timeout=10)
    assert host.returncode == 0, host.stderr
    with sqlite3.connect(root / 'host-state/rate-history.sqlite3') as journal:
        assert journal.execute('PRAGMA user_version').fetchone()[0] == 2
    invalid = root / 'invalid-startup'
    result = subprocess.run([str(release / 'bin/framerize'), 'daemon',
        '--listen', 'not-an-address', '--db', str(invalid / 'accounts.db'),
        '--profiles', str(invalid / 'profiles')], cwd=root, env=env,
        capture_output=True, text=True, timeout=10)
    assert result.returncode != 0 and 'bad --listen' in result.stderr
    assert not invalid.exists(), 'invalid startup created application state'
    for signum in (signal.SIGTERM, signal.SIGINT):
        lifecycle(release, root, env, signum)
    verify(release)
    print(json.dumps({'result': 'pass', 'commit': info['commit'], 'root': str(root),
        'archive_sha256': sha256(archive),
        'checks': ['archive', 'runtime', 'host-shutdown', 'invalid-startup',
            'startup-and-ui', 'profile-exclusion', 'SIGTERM', 'SIGINT', 'profile-reuse']}))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('archive', type=Path)
    args = parser.parse_args()
    main(args.archive.resolve())
