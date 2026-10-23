"""Real-server transport for synthetic restored-account process tests."""
import ctypes
import http.client
import json
import os
from pathlib import Path
import shlex
import signal
import socket
import sqlite3
import subprocess
import time

from restored_native_host import Lab, sha


class DaemonLab(Lab):
    def __init__(self, root, package, python, binary, routing):
        self.daemons = set()
        super().__init__(root, package, python)
        self.binary, self.routing = Path(binary), Path(routing)
        assert ctypes.CDLL(None).prctl(36, 1, 0, 0, 0) == 0, 'cannot own orphan cleanup'

    def state(self, name, full=False, changed=False):
        state = super().state(name, counts=(180 if full else 0,))
        root, account = state['root'], state['accounts'][0]
        record = dict(id=account, alias='fixture', framer_user_id='fixture-user-0',
            castle_device_id='fixture-selected', current_team='fixture-team',
            current_project='fixture-project', state='ready', credit=None,
            consecutive_failures=0, cooldown_step=0)
        with sqlite3.connect(root / 'framerize.db') as db:
            db.execute('UPDATE accounts SET json=?', (json.dumps(record),))
        state['inventory'], _ = self.support.inventory.collect(root)
        pending = root / 'cooldown-handoff/pending.json'
        request = json.loads(pending.read_text())
        request['inventory'] = state['inventory']
        request['destination']['binary_sha256'] = sha(self.binary)
        # Finalize only this newly created synthetic handoff, before startup.
        pending.unlink()
        state['pending_hash'] = self.support.private_state.write_new(pending, request)
        if changed:
            with sqlite3.connect(root / 'framerize.db') as db:
                db.execute('UPDATE accounts SET json=?',
                    (json.dumps(dict(record, alias='changed-fixture')),))
        state['launch_inventory'], _ = self.support.inventory.collect(root)
        node, entry = root / 'worker-sentinel', root / 'unused-worker-entry'
        node.write_text('#!/bin/sh\nprintf unexpected > ' +
            shlex.quote(str(root / 'unexpected-worker')) + '\nexit 97\n')
        node.chmod(0o700)
        entry.write_text('This fixture must refuse before starting any worker.\n')
        config = dict(node=str(node), entry=str(entry), pins=[
            dict(path=str(path), sha256=sha(path)) for path in (node, entry)])
        (root / 'worker.json').write_text(json.dumps(config))
        with socket.socket() as probe:
            probe.bind(('127.0.0.1', 0))
            state['port'] = probe.getsockname()[1]
        return state

    def start_daemon(self, state):
        root = state['root']
        env = {key: os.environ[key] for key in ('PATH', 'HOME', 'LANG', 'INVOCATION_ID')}
        env.update(FRAMERIZE_PYTHON=str(self.python),
            FRAMERIZE_GATE_STATE=str(root / 'gate.json'), FRAMERIZE_DEVICE_POOL='1',
            FRAMERIZE_RESERVE_PER_DAY='0', TOKIO_WORKER_THREADS='2',
            PYTHONDONTWRITEBYTECODE='1')
        argv = [str(self.binary), 'daemon', '--listen', '127.0.0.1:' + str(state['port']),
            '--db', str(root / 'framerize.db'), '--profiles', str(root / 'profiles'),
            '--web-dir', str(root / 'web'), '--engine', 'camoufox',
            '--sidecar-script', str(self.package / 'camoufox_host.py'),
            '--active-pool-size', '1', '--eligible-accounts', state['accounts'][0],
            '--offline-worker-config', str(root / 'worker.json'),
            '--offline-routing-config', str(self.routing)]
        with (root / 'server.log').open('xb') as log:
            child = subprocess.Popen(argv, env=env, stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL, stderr=log, start_new_session=True)
        self.children.append(child)
        self.daemons.add(child.pid)
        state['daemon'] = self.support.startup_state.process(child.pid)
        deadline = time.monotonic() + 10
        while True:
            assert child.poll() is None, 'server exited during startup'
            try:
                if self.request(state, '/api/health')[:2] == (200, b'ok'):
                    return child
            except OSError:
                pass
            assert time.monotonic() < deadline, 'server startup deadline'
            time.sleep(0.05)

    def request(self, state, path, payload=None, method=None):
        connection = http.client.HTTPConnection('127.0.0.1', state['port'], timeout=12)
        headers = {'Connection': 'close', 'Content-Type': 'application/json',
            'x-framerize-account': state['accounts'][0],
            'x-framerize-session': 'synthetic-restored-daemon'}
        if payload is not None:
            headers['x-framerize-context'] = json.dumps({
                'page_url': 'https://framer.com/projects/fixture-project',
                'attempts': [{'feature_category': 'fixture', 'feature_name': 'fixture',
                    'step': 'fixture', 'turn_id': 'fixture', 'user_prompt': 'fixture only'}]})
        try:
            connection.request(method or ('POST' if payload is not None else 'GET'), path,
                body=None if payload is None else json.dumps(payload), headers=headers)
            with connection.getresponse() as response:
                body = response.read(131073)
                assert len(body) <= 131072, 'fixture response exceeds bound'
                return response.status, body, dict(response.getheaders())
        finally:
            connection.close()

    def unchanged(self, state):
        assert self.support.inventory.collect(state['root'])[0] == state['launch_inventory']
        assert sha(state['root'] / 'cooldown-handoff/pending.json') == state['pending_hash']
        assert not (state['root'] / 'unexpected-worker').exists(), 'worker started'
        assert not list((state['root'] / 'profiles/.sessions').iterdir()), 'session state written'

    def witness(self, state):
        path = state['root'] / 'cooldown-handoff' / ('restored-' + state['nonce'] + '.json')
        receipt = json.loads(path.read_text())
        assert receipt['main'] == state['daemon'], 'restoration did not bind the real server'
        assert receipt['request_sha256'] == state['pending_hash']
        assert receipt['snapshot_sha256'] == state['snapshot_hash']
        assert receipt['account_count'] == 1
        assert receipt['invocation_id'] == os.environ['INVOCATION_ID']
        host = receipt['sidecar']
        assert self.support.startup_state.process(host['pid']) == host
        def descendants(pid):
            rows = Path('/proc', str(pid), 'task').glob('*/children')
            return set().union(*(set(path.read_text().split()) for path in rows))
        assert descendants(state['daemon']['pid']) == {str(host['pid'])}
        assert not descendants(host['pid']), 'host started a browser or worker'
        return dict(main=receipt['main'], sidecar=host, generation=receipt['generation'],
            request_sha256=receipt['request_sha256'], snapshot_sha256=receipt['snapshot_sha256'])

    def stop(self, child):
        daemon = child.pid in self.daemons
        try:
            super().stop(child)
        finally:
            if daemon:
                self.daemons.remove(child.pid)
                self.reap(child.pid)

    def reap(self, group):
        deadline, forced = time.monotonic() + 5, False
        while True:
            try:
                pid, status = os.waitpid(-1, os.WNOHANG)
            except ChildProcessError:
                break
            if pid:
                self.joins.append(dict(pid=pid, returncode=os.waitstatus_to_exitcode(status),
                    adopted=True, forced=forced))
                continue
            if time.monotonic() >= deadline:
                assert not forced, 'descendant cleanup deadline'
                try:
                    os.killpg(group, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                deadline, forced = time.monotonic() + 5, True
            time.sleep(0.02)
        assert not forced, 'fixture descendant required forced cleanup'
