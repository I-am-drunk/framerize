"""Startup regression fixtures. No SDK, accounts, service or network is used."""
import hashlib
import json
import os
from pathlib import Path
import sys
import tempfile
import textwrap
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
from cooldown_support import startup_state
from prepare_browser_runtime import prepare


class Host:
    def __init__(self):
        self.accounts, self.lifecycles = {}, {}
        self.generation = 'a' * 32

    async def _initialize_account(self, *args):
        raise AssertionError('no browser fixture may start')

    async def handle(self, *args):
        raise AssertionError('no request fixture may start')

    async def _use_account(self, *args):
        raise AssertionError('no account fixture may start')

    def _browser_status(self, *args):
        raise AssertionError('no runtime fixture may be queried')

    async def _finish_close(self, *args):
        raise AssertionError('no browser fixture may close')


class RuntimeIsolation(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.code = self.root / 'shared-code'
        self.instance = self.root / 'isolated-state'
        self.devices = self.instance / 'profiles/.sidecar/camoufox'
        self.devices.mkdir(parents=True)
        self.package = prepare(self.code)

    def pending(self, root):
        directory = root / 'cooldown-handoff'
        directory.mkdir(mode=0o700)
        path = directory / 'pending.json'
        path.write_text('{incomplete fixture')
        path.chmod(0o600)
        return path

    def install(self, *, legacy=False):
        prefix = '_framerize_cooldown'
        prior = {key: value for key, value in sys.modules.items() if key.startswith(prefix)}
        for key in prior:
            del sys.modules[key]
        def restore_imports():
            for key in list(sys.modules):
                if key.startswith(prefix):
                    del sys.modules[key]
            sys.modules.update(prior)
        self.addCleanup(restore_imports)
        script = self.code / 'camoufox_host.py'
        source = script.read_text()
        start = source.index('    # Restore an explicitly armed, one-use cooldown transfer')
        stop = source.index('    asyncio.run(Host().loop())', start)
        hook = textwrap.dedent(source[start:stop])
        if legacy:
            hook = hook.replace('_instance_root, Path(__file__).resolve()',
                                'Path(__file__).resolve().parent, Path(__file__).resolve()')
        class InstanceHost(Host):
            pass
        namespace = dict(Host=InstanceHost, AccountLifecycle=SimpleNamespace,
            ROOT=str(self.devices), Path=Path, sys=sys, os=os, hashlib=hashlib, json=json,
            __file__=str(script), MINT_GAP_S=2.5, DEV_WINDOW_S=600,
            DEV_WINDOW_CAP=180, DEV_COOLDOWN_S=600)
        exec(compile(hook, str(script), 'exec'), namespace)
        return InstanceHost

    def test_parallel_host_never_reads_shared_pending_handoff(self):
        pending = self.pending(self.code)
        original = pending.read_bytes()
        host = self.install()()
        self.assertEqual(host.accounts, {})
        self.assertEqual(host.lifecycles, {})
        self.assertFalse(hasattr(host, '_cooldown_restoration'))
        self.assertEqual(pending.read_bytes(), original)
        self.assertFalse((self.instance / 'cooldown-handoff').exists())

    def test_original_script_directory_binding_reproduces_failure(self):
        self.pending(self.code)
        with self.assertRaises(json.JSONDecodeError):
            self.install(legacy=True)()

    def test_bad_handoff_in_own_instance_still_fails_closed(self):
        pending = self.pending(self.instance)
        original = pending.read_bytes()
        with self.assertRaises(json.JSONDecodeError):
            self.install()()
        self.assertEqual(pending.read_bytes(), original)

    def test_same_instance_account_change_does_not_restore(self):
        self.pending(self.instance)
        request = dict(inventory={'records': {'fixture': 'before'}},
                       snapshot={'sha256': 'snapshot-hash'})
        with patch.object(startup_state.private_state, 'read',
                side_effect=[(request, 'request-hash'), ({}, 'snapshot-hash')]), \
             patch.object(startup_state, 'checked_request', return_value=(None, None, None, None)), \
             patch.object(startup_state.inventory, 'collect',
                return_value=({'records': {'fixture': 'after'}}, {})), \
             patch.object(startup_state, 'commit_restore') as restore:
            with self.assertRaisesRegex(ValueError, '^saved account state changed$'):
                startup_state.restore_host(Host(), SimpleNamespace, self.instance, self.code)
            restore.assert_not_called()

    def test_runtime_root_requires_canonical_instance_layout(self):
        self.assertEqual(startup_state.runtime_root(self.devices), self.instance)
        for path in ('profiles/.sidecar/camoufox', self.root / 'other/camoufox'):
            with self.subTest(path=str(path)), self.assertRaises(ValueError):
                startup_state.runtime_root(path)
        alias = self.root / 'alias'
        alias.symlink_to(self.instance, target_is_directory=True)
        with self.assertRaises(ValueError):
            startup_state.runtime_root(alias / 'profiles/.sidecar/camoufox')


if __name__ == '__main__':
    unittest.main()
