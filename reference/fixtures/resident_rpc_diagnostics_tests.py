"""Fixed RPC labels with synthetic ports; no browser or runtime imports."""
import ast
from pathlib import Path
import sys
import types
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import resident_host_activity_tests as fixture

SOURCE = Path(__file__).resolve().parents[2] / 'scripts/resident_authorization.py'
node = next(n for n in ast.parse(SOURCE.read_text()).body
            if isinstance(n, ast.FunctionDef) and n.name == 'rpc_failure_facts')
namespace = {}
exec(compile(ast.Module(body=[node], type_ignores=[]), str(SOURCE), 'exec'), namespace)
facts = namespace['rpc_failure_facts']


class RpcDiagnosticTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.f = fixture.HostActivityTests()
        await self.f.asyncSetUp()
        self.events = []
        self.f.ns['emit'] = lambda *args, **kw: self.events.append((args, kw))
        self.f.host._resident_control = object()

    async def asyncTearDown(self):
        await self.f.asyncTearDown()

    def test_labels_are_fixed_and_do_not_export_exception_text(self):
        call = types.SimpleNamespace(__name__='evaluate')
        for raw, label in [('Execution context was destroyed', 'navigation'),
                           ('Target closed', 'closed'), ('error loading a module', 'module_load'),
                           ('Timeout', 'timeout'), ('private token cookie body', 'other')]:
            self.assertEqual(facts(call, RuntimeError(raw)), dict(method='evaluate', category=label))

    def test_unknown_methods_and_supplied_error_properties_are_ignored(self):
        error = RuntimeError('opaque synthetic failure')
        error.category, error.method = 'private-value', 'private-value'
        self.assertEqual(facts(types.SimpleNamespace(__name__='private-call'), error),
                         dict(method='other', category='other'))

    async def fail_rpc(self, observer=facts):
        port = self.f.port()
        port.__name__ = 'evaluate'
        port.error = RuntimeError('Timeout private-token')
        module = types.ModuleType('resident_authorization')
        module.rpc_failure_facts = observer
        with patch.dict(sys.modules, resident_authorization=module):
            with self.assertRaises(RuntimeError) as caught:
                await self.f.host._rpc('a', port)
        self.assertIs(caught.exception, port.error)
        self.assertEqual(len(port.calls), 1)
        self.assertIsNotNone(self.f.acc.lifecycle.resident_uncertain)
        self.f.refused()

    async def test_event_preserves_the_original_failure_and_fence(self):
        await self.fail_rpc()
        self.assertEqual(self.events, [(('resident.rpc.failed',),
                                      dict(method='evaluate', category='timeout'))])

    async def test_observer_failure_does_not_replace_the_rpc_error(self):
        def broken(*args):
            raise ValueError('synthetic observer failure')
        await self.fail_rpc(broken)
        self.assertEqual(self.events, [])

    async def test_default_host_does_not_emit_resident_diagnostics(self):
        self.f.host._resident_control = None
        await self.fail_rpc()
        self.assertEqual(self.events, [])


if __name__ == '__main__':
    unittest.main()
