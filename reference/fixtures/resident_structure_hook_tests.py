"""Inert tests for attaching diagnostics to the original resident mint."""
import asyncio
import importlib.util
from pathlib import Path
import unittest

path = Path(__file__).resolve().parents[2] / 'scripts/experiments/resident_structure_hook.py'
spec = importlib.util.spec_from_file_location('resident_structure_hook', path)
HOOK = importlib.util.module_from_spec(spec)
spec.loader.exec_module(HOOK)

class HookTests(unittest.IsolatedAsyncioTestCase):
    async def test_success_preserves_original_result_and_calls_once(self):
        token, calls, records = object(), [], []
        class Operation:
            phase = 'baseline_pending'
            identity = {'lease': 'lease'}
            async def charged_mint(self):
                calls.append('original'); return token
        original = Operation.charged_mint
        describe, emit = object(), object()
        restore = HOOK.install(Operation, lambda *args: records.append(args), describe, emit)
        self.assertIs(await Operation().charged_mint(), token)
        self.assertEqual(calls, ['original'])
        self.assertEqual(records, [(token, 'lease', 'baseline', describe, emit)])
        self.assertTrue(restore())
        self.assertIs(Operation.charged_mint, original)

    async def test_observer_failure_cannot_replace_original_result(self):
        token = object()
        class Operation:
            phase = 'native_pending'
            identity = {'lease': 'lease'}
            async def charged_mint(self): return token
        def record(*args): raise RuntimeError('diagnostic failed')
        HOOK.install(Operation, record, None, None)
        self.assertIs(await Operation().charged_mint(), token)

    async def test_original_failure_propagates_without_observation(self):
        failure, records = RuntimeError('original'), []
        class Operation:
            async def charged_mint(self): raise failure
        HOOK.install(Operation, lambda *args: records.append(args), None, None)
        with self.assertRaises(RuntimeError) as caught:
            await Operation().charged_mint()
        self.assertIs(caught.exception, failure)
        self.assertEqual(records, [])

    async def test_cancellation_reaches_original_without_observation(self):
        started, cancelled, records = asyncio.Event(), asyncio.Event(), []
        class Operation:
            async def charged_mint(self):
                started.set()
                try: await asyncio.Event().wait()
                finally: cancelled.set()
        HOOK.install(Operation, lambda *args: records.append(args), None, None)
        task = asyncio.create_task(Operation().charged_mint())
        await started.wait(); task.cancel()
        with self.assertRaises(asyncio.CancelledError): await task
        self.assertTrue(cancelled.is_set())
        self.assertEqual(records, [])

    async def test_restore_never_overwrites_a_later_owner(self):
        class Operation:
            async def charged_mint(self): return 'synthetic'
        restore = HOOK.install(Operation, None, None, None)
        async def successor(self): return 'successor'
        Operation.charged_mint = successor
        self.assertFalse(restore())
        self.assertIs(Operation.charged_mint, successor)

if __name__ == '__main__':
    unittest.main()
