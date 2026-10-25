"""Release imports and state isolation using temporary synthetic state only."""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

RUNTIME = Path(__file__).resolve().parents[1]

class PackageTests(unittest.TestCase):
    def run_code(self, root, code, state_name='profiles/.sidecar/camoufox'):
        with tempfile.TemporaryDirectory() as directory:
            state = Path(directory) / state_name
            state.mkdir(parents=True)
            return subprocess.run([sys.executable, '-I', '-B', '-c',
                'import sys; sys.path.insert(0, ' + repr(str(root)) + ');\n' + code],
                env={'PATH': os.defpath, 'HOME': directory, 'FRAMERIZE_STATE': state_name},
                cwd=directory, input='', capture_output=True, text=True, timeout=10)

    def test_confirmed_close_preserves_primary_device_for_reopen(self):
        result = self.run_code(RUNTIME, '''
import asyncio
from types import SimpleNamespace
from unittest.mock import AsyncMock
from framerize_host.app import Host, AccountLifecycle
from framerize_host.restoration.startup_state import restore_primary
async def check():
    host = Host()
    life = AccountLifecycle()
    life.browser = object()
    life.close_receipt = SimpleNamespace(browser=life.browser)
    acc = SimpleNamespace(primary_dev='chosen')
    host.lifecycles['fixture'] = life
    host.accounts['fixture'] = acc
    host._dispose_account = AsyncMock()
    assert await host._finish_close('fixture', life, acc)
    reopened = SimpleNamespace(lifecycle=life, devices=['first', 'chosen'], primary_dev='first')
    restore_primary(reopened)
    assert reopened.primary_dev == 'chosen'
    await host.close()
asyncio.run(check())
''')
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_entrypoint_accepts_relative_and_custom_profile_paths(self):
        for state in ('profiles/.sidecar/camoufox', 'custom/engine-state'):
            with self.subTest(state=state):
                code = 'from framerize_host.__main__ import main\nmain(' + repr(str(RUNTIME / 'host.py')) + ')'
                result = self.run_code(RUNTIME, code, state)
                self.assertEqual(result.returncode, 0, result.stderr)

    def test_browser_imports_are_lazy_and_native_adapter_is_absent(self):
        result = self.run_code(RUNTIME, '''
import asyncio, builtins, sys
original = builtins.__import__
def checked(name, *args, **kwargs):
    if name.split('.')[0] in ('camoufox', 'playwright'):
        raise AssertionError('eager browser import')
    return original(name, *args, **kwargs)
builtins.__import__ = checked
from framerize_host.app import Host
host = Host()
assert host._browser_status('fixture')['state'] == 'absent'
assert not any(n.startswith('framerize_native') for n in sys.modules)
asyncio.run(host.close())
''')
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_incomplete_package_refuses_instead_of_changing_behavior(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            shutil.copytree(RUNTIME / 'framerize_host', root / 'framerize_host')
            (root / 'framerize_host/castle_forge.py').unlink()
            result = self.run_code(root, 'from framerize_host.app import Host')
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('castle_forge', result.stderr)

if __name__ == '__main__':
    unittest.main()
