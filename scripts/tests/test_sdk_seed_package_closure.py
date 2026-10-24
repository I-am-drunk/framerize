"""Prepared-host closure checks. Startup uses only isolated synthetic modules."""
import builtins
import hashlib
import json
import os
from pathlib import Path
import runpy
import sys
import tempfile
import textwrap
from types import ModuleType
import unittest
from unittest.mock import patch


SCRIPTS = Path(__file__).resolve().parents[1]
COOLDOWN = ('__init__.py', 'startup_state.py', 'inventory.py', 'ledger.py',
            'private_state.py')
GLOBALS = ('resident_activity', 'contextual_authorization',
           'resident_authorization', 'resident_bootstrap', 'sdk_seed_observation')
SOURCES = {name: 'cooldown_support/' + name for name in COOLDOWN}
SOURCES.update({name + '.py': name + '.py' for name in (*GLOBALS, 'native_device')})
HOST = '''\
import asyncio, hashlib, json, os, sys
from pathlib import Path
import _package_fixture as fixture
ROOT = str(fixture.root / 'profiles/.sidecar/camoufox')
MINT_GAP_S, DEV_WINDOW_S, DEV_WINDOW_CAP, DEV_COOLDOWN_S = 2.5, 600, 180, 600
AccountLifecycle = object
class Host:
    _sdk_seed, _sdk_seed_enabled = None, False
    def __init__(self):
        fixture.events.append('host_init')
    async def loop(self):
        assert self._sdk_seed_enabled
        assert self._sdk_seed is sys.modules['sdk_seed_observation']
        fixture.events.append('loop')
if __name__ == '__main__':
    asyncio.run(Host().loop())
'''


class SdkSeedPackageClosure(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()

    def write(self, path, source):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(textwrap.dedent(source))

    def prepare(self, scripts, output):
        namespace = runpy.run_path(str(scripts / 'prepare_browser_runtime.py'))
        return namespace['prepare'](output)

    def fixture_sources(self):
        scripts = self.root / 'source'
        scripts.mkdir()
        (scripts / 'prepare_browser_runtime.py').write_bytes(
            (SCRIPTS / 'prepare_browser_runtime.py').read_bytes())
        self.write(scripts / 'camoufox_host.py', HOST)
        support = scripts / 'cooldown_support'
        support.mkdir()
        (support / 'startup_hook.txt').write_bytes(
            (SCRIPTS / 'cooldown_support/startup_hook.txt').read_bytes())
        for name in (*GLOBALS[:-1], 'inventory', 'ledger', 'private_state'):
            target = scripts if name in GLOBALS else support
            self.write(target / (name + '.py'),
                       f'from _package_fixture import events\nevents.append("load:{name}")\n')
        with (scripts / 'contextual_authorization.py').open('a') as source:
            source.write('login_cookie = object()\n')
        self.write(scripts / 'sdk_seed_observation.py', '''\
            from contextual_authorization import login_cookie
            from _package_fixture import events
            events.append('load:sdk_seed_observation')
        ''')
        self.write(support / '__init__.py', 'from . import startup_state\n')
        self.write(support / 'startup_state.py', '''\
            from pathlib import Path
            from . import inventory, ledger, private_state
            import _package_fixture as fixture
            def runtime_root(root):
                return Path(root).parents[2]
            def install(host, lifecycle, root, script):
                fixture.startup_args = (host, lifecycle, root, script)
                fixture.events.append('cooldown_install')
        ''')
        self.write(scripts / 'native_device.py', '''\
            import sdk_seed_observation as sdk_seed
            import _package_fixture as fixture
            fixture.events.append('load:native_device')
            def install(host, root, **options):
                host._sdk_seed = sdk_seed
                host._sdk_seed_enabled = True
                fixture.native_args = (host, root, options)
                fixture.events.append('native_install')
                fixture.after_install()
        ''')
        return scripts

    def prepared_fixture(self):
        output = self.root / 'package'
        result = self.prepare(self.fixture_sources(), output)
        support = output / ('cooldown-support-' + result['support_sha256'][:16])
        return output, support

    def run_fixture(self, output, fixture):
        original_import = builtins.__import__
        def checked_import(name, *args, **kwargs):
            if name in GLOBALS and name not in sys.modules:
                raise AssertionError('support dependency was not loaded: ' + name)
            return original_import(name, *args, **kwargs)
        with patch.dict(sys.modules), patch.object(sys, 'dont_write_bytecode', True):
            for name in list(sys.modules):
                if name in GLOBALS or name.split('.')[0] == '_framerize_cooldown':
                    del sys.modules[name]
            sys.modules['_package_fixture'] = fixture
            with patch('builtins.__import__', checked_import):
                runpy.run_path(str(output / 'camoufox_host.py'), run_name='__main__')

    def tracker(self):
        fixture = ModuleType('_package_fixture')
        fixture.root = self.root / 'untouched-state'
        fixture.events = []
        fixture.after_install = lambda: None
        return fixture

    def test_prepared_support_copies_and_pins_exact_sources(self):
        output = self.root / 'real-source-package'
        result = self.prepare(SCRIPTS, output)
        support = output / ('cooldown-support-' + result['support_sha256'][:16])
        manifest = (support / 'runtime-sources.json').read_bytes()
        pins = json.loads(manifest)
        self.assertEqual(hashlib.sha256(manifest).hexdigest(), result['support_sha256'])
        self.assertEqual(set(pins), set(SOURCES))
        self.assertEqual({path.name for path in support.iterdir()},
                         set(SOURCES) | {'runtime-sources.json'})
        for name, relative in SOURCES.items():
            with self.subTest(source=name):
                expected = (SCRIPTS / relative).read_bytes()
                self.assertEqual((support / name).read_bytes(), expected)
                self.assertEqual(pins[name], hashlib.sha256(expected).hexdigest())
        self.assertEqual(result['sidecar_sha256'], hashlib.sha256(
            (output / 'camoufox_host.py').read_bytes()).hexdigest())

    def test_helper_is_loaded_before_native_install_and_host_loop(self):
        output, _ = self.prepared_fixture()
        for enrollment in ('0', '1'):
            with self.subTest(enrollment=enrollment), patch.dict(
                    os.environ, FRAMERIZE_NATIVE_ENROLLMENT=enrollment):
                fixture = self.tracker()
                self.run_fixture(output, fixture)
                self.assertEqual(fixture.events, [
                    *('load:' + name for name in GLOBALS),
                    'load:inventory', 'load:ledger', 'load:private_state',
                    'cooldown_install', 'load:native_device', 'native_install',
                    'host_init', 'loop'])
                host, lifecycle, root, script = fixture.startup_args
                self.assertEqual(root, fixture.root)
                self.assertEqual(script, output / 'camoufox_host.py')
                native_host, native_root, options = fixture.native_args
                self.assertIs(native_host, host)
                self.assertEqual(native_root, str(root / 'profiles/.sidecar/camoufox'))
                self.assertEqual(options, dict(gap=2.5, window=600, capacity=180,
                    cooldown=600, lifecycle_type=lifecycle if enrollment == '1' else None))
                self.assertFalse(root.exists())

    def test_missing_source_helper_refuses_before_creating_package(self):
        scripts = self.fixture_sources()
        (scripts / 'sdk_seed_observation.py').unlink()
        output = self.root / 'refused-package'
        with self.assertRaises(FileNotFoundError) as error:
            self.prepare(scripts, output)
        self.assertEqual(error.exception.filename, str(scripts / 'sdk_seed_observation.py'))
        self.assertFalse(output.exists())

    def test_missing_packaged_helper_refuses_before_loading_support(self):
        output, support = self.prepared_fixture()
        (support / 'sdk_seed_observation.py').unlink()
        fixture = self.tracker()
        with self.assertRaises(FileNotFoundError) as error:
            self.run_fixture(output, fixture)
        self.assertEqual(error.exception.filename, str(support / 'sdk_seed_observation.py'))
        self.assertEqual(fixture.events, [])
        self.assertFalse(fixture.root.exists())

    def test_preload_helper_tamper_refuses_before_loading_support(self):
        output, support = self.prepared_fixture()
        helper = support / 'sdk_seed_observation.py'
        helper.write_bytes(helper.read_bytes() + b'\n# altered fixture\n')
        fixture = self.tracker()
        with self.assertRaisesRegex(RuntimeError, '^cooldown support source changed$'):
            self.run_fixture(output, fixture)
        self.assertEqual(fixture.events, [])
        self.assertFalse(fixture.root.exists())

    def test_postload_helper_tamper_refuses_before_host_construction(self):
        output, support = self.prepared_fixture()
        helper = support / 'sdk_seed_observation.py'
        original = helper.read_bytes()
        fixture = self.tracker()
        fixture.after_install = lambda: helper.write_bytes(original + b'\n# altered fixture\n')
        with self.assertRaisesRegex(RuntimeError, '^cooldown support changed while loading$'):
            self.run_fixture(output, fixture)
        self.assertEqual(helper.read_bytes(), original + b'\n# altered fixture\n')
        self.assertIn('native_install', fixture.events)
        self.assertNotIn('host_init', fixture.events)
        self.assertNotIn('loop', fixture.events)
        self.assertFalse(fixture.root.exists())


if __name__ == '__main__':
    unittest.main()
