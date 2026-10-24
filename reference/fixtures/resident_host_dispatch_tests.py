"""Inert dispatch/startup tests with no browser or filesystem operations."""
import ast
import asyncio
from contextlib import asynccontextmanager
from pathlib import Path
import sys
import types
import unittest
import uuid

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))


class ResidentHostDispatchTests(unittest.IsolatedAsyncioTestCase):
    def host(self, generation=None):
        env = {} if generation is None else {
            'FRAMERIZE_RESIDENT_APPLICATION_GENERATION': generation}
        source = ROOT / 'scripts/camoufox_host.py'
        node = next(n for n in ast.parse(source.read_text()).body
                    if isinstance(n, ast.ClassDef) and n.name == 'Host')
        namespace = dict(asyncio=asyncio, uuid=uuid, ROOT='synthetic-root',
            asynccontextmanager=asynccontextmanager,
            os=types.SimpleNamespace(environ=env, makedirs=lambda *a, **kw: None))
        exec(compile(ast.Module(body=[node], type_ignores=[]), str(source), 'exec'), namespace)
        return namespace['Host']()

    async def test_default_startup_has_no_resident_control(self):
        host = self.host()
        self.assertIsNone(host._resident_control)
        with self.assertRaisesRegex(RuntimeError, 'unavailable'):
            await host.handle({'cmd': 'resident_authorization_once'})
        self.assertFalse(host.accounts)
        self.assertFalse(host.lifecycles)

    async def test_positive_generation_binds_one_control(self):
        host = self.host('37')
        self.assertEqual(host.resident_application_generation, 37)
        self.assertIs(host._resident_control.host, host)
        self.assertEqual(host._resident_control.host_generation, host.generation)

    async def test_malformed_generation_fails_closed(self):
        for value in ('', '0', '-1', '1.0', 'true', ' 1', '１', str(2**64)):
            with self.subTest(value=value), self.assertRaisesRegex(RuntimeError, 'startup'):
                self.host(value)

    async def test_private_dispatch_uses_retained_control_before_activity(self):
        host = self.host('1')
        request = {'cmd': 'resident_authorization_once', 'action': 'discard'}
        calls = []
        async def command(req):
            calls.append(req)
            return {'state': 'fenced'}
        host._resident_control.command = command
        self.assertEqual(await host.handle(request), {'state': 'fenced'})
        self.assertEqual(calls, [request])
        self.assertFalse(host.lifecycles)

    async def test_observed_host_cannot_dispatch_resident_control(self):
        host = self.host('1')
        host._observed = object()
        with self.assertRaisesRegex(RuntimeError, 'unavailable'):
            await host.handle({'cmd': 'resident_authorization_once'})
        self.assertFalse(host.lifecycles)


if __name__ == '__main__':
    unittest.main()
