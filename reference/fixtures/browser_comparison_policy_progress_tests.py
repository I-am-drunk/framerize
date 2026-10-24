"""Policy progress is diagnostic only, including when its RPC outlives a waiter."""
import asyncio
import importlib.util
import json
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    '_policy_fixture', Path(__file__).with_name('browser_comparison_policy_tests.py'))
P = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(P)
C = P.C


class PolicyProgressTests(unittest.IsolatedAsyncioTestCase):
    asyncSetUp = P.BrowserComparisonPolicyTests.asyncSetUp
    asyncTearDown = P.BrowserComparisonPolicyTests.asyncTearDown

    def fixture(self):
        value = P.PolicyHarness()
        self.values.append(value)
        value.page.remove_listener = lambda event, cb: value.page.events[event].remove(cb)
        return value

    def send(self, value, phase, *, kind='debug', prefix=None):
        key = value.policy_args['progress_key'] if prefix is None else prefix
        for callback in tuple(value.page.events.get('console', ())):
            callback(SimpleNamespace(type=kind, text=key + phase))

    def stages(self, value):
        return [record['stage'] for record in value.progress
                if record['stage'] in C.POLICY_PROGRESS]

    async def test_only_fixed_ordered_scoped_markers_leave_the_page(self):
        value = self.fixture()
        def during():
            self.send(value, 'private-data')
            self.send(value, 'policy_eval_entry', prefix='wrong:')
            self.send(value, 'policy_eval_entry', kind='error')
            for phase in C.POLICY_PROGRESS:
                self.send(value, phase)
                self.send(value, phase)
        value.after_policy = during
        await value.prepare_context()
        self.assertEqual(self.stages(value), list(C.POLICY_PROGRESS))
        self.assertEqual(value.page.events['console'], [])
        self.assertNotIn('private-data', json.dumps(value.progress))
        self.assertNotIn(value.policy_args['progress_key'], json.dumps(value.progress))
        self.assertEqual((value.policy_calls, value.snapshots, value.mints), (1, 1, 0))

    async def test_timeout_keeps_rpc_owned_and_late_events_cannot_replace_failure(self):
        value = self.fixture()
        value.policy_release.clear()
        with patch.object(C, 'COMPARISON_POLICY_SECONDS', 0.05):
            task = asyncio.create_task(value.prepare_context())
            self.tasks.append(task)
            await value.policy_started.wait()
            listener = value.page.events['console'][0]
            self.send(value, 'policy_eval_entry')
            self.send(value, 'policy_engine_import')
            with self.assertRaises(RuntimeError):
                await task
        self.assertEqual(value.events[0]['stage'], 'policy_rpc_timeout')
        self.assertEqual(self.stages(value), ['policy_eval_entry', 'policy_engine_import'])
        self.assertTrue(value.scope.pending)
        self.assertFalse(value.policy_cancelled)
        self.assertEqual(value.page.events['console'], [])
        before = list(value.progress)
        listener(SimpleNamespace(type='debug', text=value.policy_args['progress_key'] + 'policy_call'))
        value.policy_release.set()
        await value.scope.drain()
        self.assertEqual(value.progress, before)
        self.assertEqual((value.snapshots, value.mints, value.fetches), (0, 0, 0))

    async def test_cancelled_waiter_removes_listener_without_cancelling_rpc(self):
        value = self.fixture()
        value.policy_release.clear()
        task = asyncio.create_task(value.prepare_context())
        self.tasks.append(task)
        await value.policy_started.wait()
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertEqual(value.page.events['console'], [])
        self.assertTrue(value.scope.pending)
        self.assertFalse(value.policy_cancelled)

    async def test_telemetry_failure_and_false_success_markers_cannot_accept_disabled(self):
        value = self.fixture()
        value.policy_reply.update(stage='policy_disabled', ai_disabled=True)
        value.after_policy = lambda: self.send(value, 'policy_evaluated')
        value.page.remove_listener = lambda *_: (_ for _ in ()).throw(RuntimeError('private'))
        with self.assertRaises(RuntimeError):
            await value.prepare_context()
        self.assertEqual(value.events[0]['stage'], 'policy_disabled')
        before = list(value.progress)
        self.send(value, 'policy_evaluated')
        self.assertEqual(value.progress, before)
        self.assertEqual((value.snapshots, value.mints, value.fetches), (0, 0, 0))


if __name__ == '__main__':
    unittest.main()
