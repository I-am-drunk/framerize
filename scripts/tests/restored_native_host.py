"""Synthetic state and process transport for the restored native host fixture."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import selectors
import signal
import sqlite3
import subprocess
import sys
import time
import uuid


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


class Lab:
    def __init__(self, root, package, python):
        self.root, self.package, self.python = map(Path, (root, package, python))
        self.children = []
        self.joins = []
        self.number = 0
        support = next(self.package.glob('cooldown-support-*/__init__.py'))
        spec = importlib.util.spec_from_file_location('fixture_cooldown', support,
            submodule_search_locations=[str(support.parent)])
        self.support = importlib.util.module_from_spec(spec)
        sys.modules[spec.name] = self.support
        spec.loader.exec_module(self.support)
        self.parent = self.support.startup_state.process(os.getpid())
        self.group = Path('/proc/self/cgroup').read_text().strip().split('::', 1)[1]
        self.boot = Path('/proc/sys/kernel/random/boot_id').read_text().strip()
        self.prior = [self.dead_identity() for _ in range(2)]

    def dead_identity(self):
        child = subprocess.Popen([sys.executable, '-c',
            'import sys; sys.stdin.buffer.read(1)'], stdin=subprocess.PIPE,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
        self.children.append(child)
        try:
            return self.support.startup_state.process(child.pid)
        finally:
            self.stop(child)

    def state(self, name, counts=(0,), cooling=()):
        root = self.root / name
        root.mkdir(mode=0o700)
        devices = root / 'profiles/.sidecar/camoufox'
        devices.mkdir(parents=True)
        rows, accounts = {}, []
        now = time.monotonic()
        with sqlite3.connect(root / 'framerize.db') as db:
            db.execute('CREATE TABLE accounts (id TEXT PRIMARY KEY, json TEXT NOT NULL)')
            for index, count in enumerate(counts):
                account = str(uuid.UUID(int=index + 1))
                accounts.append(account)
                record = dict(id=account, alias='fixture', framer_user_id='fixture-user-' + str(index))
                db.execute('INSERT INTO accounts VALUES (?, ?)', (account, json.dumps(record)))
                directory = devices / account
                directory.mkdir()
                self.support.private_state.write_new(directory / 'framerize-devices.json',
                    ['fixture-other', 'fixture-selected'])
                rows[account] = dict(cooling={'fixture-selected': now + 500} if index in cooling else {},
                    requests={'fixture-selected': [now - 1] * count}, last_mint=now - 10,
                    reserve_day='', reserve_used=0, primary_device='fixture-selected',
                    ambient_cooling=None, ambient_requests=None)
        directory = root / 'cooldown-handoff'
        directory.mkdir(mode=0o700)
        nonce = uuid.uuid4().hex
        snapshot = directory / ('snapshot-' + nonce + '.json')
        payload = dict(format=1, boot_id=self.boot, captured_monotonic=now,
            state=dict(generation='a' * 32, accounts=rows))
        snapshot_hash = self.support.private_state.write_new(snapshot, payload)
        inventory, _ = self.support.inventory.collect(root)
        source = dict(main=self.prior[0], sidecar=self.prior[1], cgroup=self.group,
            host_generation='a' * 32, invocation_id='a' * 32)
        target = dict(binary_sha256=self.parent['exe_sha256'], controller=self.parent,
            sidecar_sha256=sha(self.package / 'camoufox_host.py'))
        request = dict(format=1, nonce=nonce, boot_id=self.boot, source=source,
            destination=target, snapshot=dict(path=str(snapshot), sha256=snapshot_hash),
            inventory=inventory)
        pending_hash = self.support.private_state.write_new(directory / 'pending.json', request)
        return dict(root=root, accounts=accounts, nonce=nonce, snapshot_hash=snapshot_hash,
            pending_hash=pending_hash, inventory=inventory)

    def start(self, state, name='host'):
        root = state['root']
        env = dict(os.environ, FRAMERIZE_STATE=str(root / 'profiles/.sidecar/camoufox'),
            FRAMERIZE_LOG_DIR=str(root / 'profiles/.logs'),
            FRAMERIZE_GATE_STATE=str(root / 'gate.json'),
            FRAMERIZE_DEVICE_POOL='1', FRAMERIZE_RESERVE_PER_DAY='0',
            PYTHONDONTWRITEBYTECODE='1')
        with (root / (name + '.stderr')).open('xb') as log:
            child = subprocess.Popen([str(self.python), '-B', str(self.package / 'camoufox_host.py')],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=log,
                env=env, start_new_session=True, bufsize=0)
        self.children.append(child)
        return child

    def command(self, child, command, **fields):
        self.number += 1
        payload = (json.dumps(dict(id=self.number, cmd=command, **fields)) + '\n').encode()
        assert len(payload) < 4096, 'fixture command exceeds bound'
        child.stdin.write(payload)
        child.stdin.flush()
        data = bytearray()
        deadline = time.monotonic() + 10
        with selectors.DefaultSelector() as ready:
            ready.register(child.stdout, selectors.EVENT_READ)
            while b'\n' not in data:
                timeout = deadline - time.monotonic()
                assert timeout > 0 and ready.select(timeout), 'host reply deadline'
                chunk = os.read(child.stdout.fileno(), 4096)
                assert chunk, 'host exited before replying'
                data.extend(chunk)
                assert len(data) <= 65536, 'host reply exceeds bound'
        reply = json.loads(data)
        assert reply['id'] == self.number and reply['ok'] is True, 'host command failed'
        self.no_children(child)
        assert child.poll() is None, 'host exited after replying'
        return reply['result']

    def no_children(self, child):
        children = set()
        for path in Path('/proc', str(child.pid), 'task').glob('*/children'):
            children.update(path.read_text().split())
        assert not children, 'generated host started an unexpected process'

    def stop(self, child):
        forced = False
        if child.poll() is None:
            try:
                os.killpg(child.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        try:
            child.wait(timeout=5)
        except subprocess.TimeoutExpired:
            forced = True
            os.killpg(child.pid, signal.SIGKILL)
            child.wait(timeout=5)
        for pipe in (child.stdin, child.stdout):
            if pipe is not None:
                pipe.close()
        self.children.remove(child)
        self.joins.append(dict(pid=child.pid, returncode=child.returncode, forced=forced))
        assert not forced, 'fixture process exceeded shutdown deadline'

    def finish(self):
        failures = []
        for child in tuple(self.children):
            try:
                self.stop(child)
            except Exception as error:
                failures.append(type(error).__name__)
        assert not failures and not self.children, 'fixture process cleanup failed'
