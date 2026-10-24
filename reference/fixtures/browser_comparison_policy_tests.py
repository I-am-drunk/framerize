"""Inert policy preflight fixtures. UNRUN by the source-only author."""
import asyncio
import copy
import importlib.util
import json
from pathlib import Path
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    '_policy_diagnostics', Path(__file__).with_name('browser_comparison_diagnostics_tests.py'))
D = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(D)
C = D.C
SECRET = 'private-policy-fixture'


def ready():
    return {'stage': 'policy_ready', 'policy_known': True, 'ai_disabled': False,
            'policy_project_match': True, 'policy_space_match': True,
            'policy_page_match': True, 'policy_device_match': True, 'policy_engine_match': True}


class PolicyHarness(D.DiagnosticHarness):
    def __init__(self):
        super().__init__()
        self.policy_started, self.policy_release = asyncio.Event(), asyncio.Event()
        self.policy_release.set()
        self.policy_calls = self.snapshots = 0
        self.policy_cancelled = False
        self.policy_reply, self.policy_error = ready(), None

    async def evaluate(self, script, args=None):
        if script == 'mw:' + C.COMPARISON_SNAPSHOT:
            self.snapshots += 1
        if script != 'mw:' + C.COMPARISON_POLICY:
            return await super().evaluate(script, args)
        self.policy_calls += 1
        self.policy_args = args
        self.calls.append(('policy',))
        self.policy_started.set()
        try:
            await self.policy_release.wait()
        except asyncio.CancelledError:
            self.policy_cancelled = True
            raise
        if self.policy_error is not None:
            raise self.policy_error
        if self.after_policy is not None:
            self.after_policy()
        return copy.deepcopy(self.policy_reply)


class BrowserComparisonPolicyTests(unittest.IsolatedAsyncioTestCase):
    asyncSetUp = D.FIXTURE.BrowserComparisonTests.asyncSetUp

    def fixture(self):
        value = PolicyHarness()
        self.values.append(value)
        return value

    async def asyncTearDown(self):
        for value in self.values:
            value.policy_release.set()
        await D.FIXTURE.BrowserComparisonTests.asyncTearDown(self)

    def assert_refused(self, value, stage):
        self.assertEqual((value.snapshots, value.mints, value.fetches), (0, 0, 0))
        self.assertIsNone(value.scope.comparison)
        self.assertEqual(value.scope.comparison_phase, 'failed')
        self.assertEqual(value.events[0]['stage'], stage)
        self.assertEqual(value.events[0]['outcome'], 'failed')
        self.assertNotIn(SECRET, json.dumps(value.events))
        self.assertNotIn(SECRET, json.dumps(value.progress))

    async def test_ready_policy_precedes_snapshot_and_preserves_one_shot_pair(self):
        value = self.fixture()
        await value.prepare_context()
        self.assertEqual(value.policy_args, {'project': D.FIXTURE.OLD.PROJECT,
                                           'page_url': D.FIXTURE.OLD.PAGE_URL})
        self.assertEqual((value.policy_calls, value.snapshots, value.mints), (1, 1, 0))
        calls = [call[0] for call in value.calls]
        self.assertLess(calls.index('dispose_readiness'), calls.index('policy'))
        self.assertLess(calls.index('policy'), calls.index('cookies'))
        self.assertIs(value.events[0]['policy_known'], True)
        self.assertIs(value.events[0]['ai_disabled'], False)
        self.assertTrue((await value.baseline())['accepted'])
        await value.native()
        self.assertEqual((value.policy_calls, value.mints, value.fetches), (1, 2, 1))

    async def test_disabled_policy_refuses_before_snapshot_and_cannot_replay(self):
        value = self.fixture()
        value.policy_reply.update(stage='policy_disabled', ai_disabled=True)
        with self.assertRaisesRegex(RuntimeError, '^browser comparison failed$'):
            await value.prepare_context()
        self.assert_refused(value, 'policy_disabled')
        self.assertIs(value.events[0]['ai_disabled'], True)
        with self.assertRaises(ValueError):
            await value.prepare_context()
        self.assertEqual(value.policy_calls, 1)

    async def test_unknown_policy_never_becomes_false_or_releases_context(self):
        for stage in ('policy_import', 'policy_engine', 'policy_space', 'policy_post_context'):
            value = self.fixture()
            value.policy_reply = {'stage': stage, 'policy_known': False}
            with self.assertRaisesRegex(RuntimeError, '^browser comparison failed$'):
                await value.prepare_context()
            self.assert_refused(value, stage)
            self.assertNotIn('ai_disabled', value.events[0])

    async def test_invalid_reply_cannot_claim_ready_or_expose_private_fields(self):
        for reply in (None, [], dict(ready(), token=SECRET), dict(ready(), stage=SECRET),
                      dict(ready(), ai_disabled=0), dict(ready(), policy_known=False),
                      dict(ready(), stage='policy_engine'),
                      {'stage': 'policy_space', 'policy_known': False, 'ai_disabled': False},
                      {'stage': 'policy_ready', 'policy_known': True}):
            value = self.fixture()
            value.policy_reply = reply
            with self.assertRaisesRegex(RuntimeError, '^browser comparison failed$') as caught:
                await value.prepare_context()
            self.assertIsNone(caught.exception.__cause__)
            self.assert_refused(value, 'policy_reply_invalid')

    async def test_settled_rpc_errors_report_fixed_categories_only(self):
        for error, stage in ((TimeoutError(SECRET), 'policy_rpc_timeout'),
                             (RuntimeError(SECRET), 'policy_rpc_error')):
            value = self.fixture()
            value.policy_error = error
            with self.assertRaisesRegex(RuntimeError, '^browser comparison failed$'):
                await value.prepare_context()
            self.assert_refused(value, stage)

    async def test_scope_refusal_cannot_publish_an_allowed_reply(self):
        value = self.fixture()
        value.after_policy = lambda: setattr(value.scope, 'failure', SECRET)
        with self.assertRaisesRegex(RuntimeError, '^browser comparison failed$'):
            await value.prepare_context()
        self.assert_refused(value, 'policy_scope_refused')

    async def test_deadline_retains_pending_rpc_without_snapshot_or_retry(self):
        value = self.fixture()
        value.policy_release.clear()
        with patch.object(C, 'COMPARISON_POLICY_SECONDS', 0.02):
            with self.assertRaisesRegex(RuntimeError, '^browser comparison failed$'):
                await value.prepare_context()
        self.assert_refused(value, 'policy_rpc_timeout')
        self.assertTrue(value.scope.pending)
        self.assertFalse(value.policy_cancelled)
        value.policy_release.set()
        await asyncio.wait_for(value.scope.drain(), 2)
        self.assertFalse(value.scope.pending)
        self.assertEqual((value.policy_calls, value.snapshots), (1, 0))

    async def test_cancelled_waiter_keeps_rpc_owned_until_settlement(self):
        value = self.fixture()
        value.policy_release.clear()
        task = asyncio.create_task(value.prepare_context())
        self.tasks.append(task)
        await asyncio.wait_for(value.policy_started.wait(), 2)
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertTrue(value.scope.pending)
        self.assertFalse(value.policy_cancelled)
        self.assertEqual((value.snapshots, value.mints, value.fetches), (0, 0, 0))
        drain = asyncio.create_task(value.scope.drain())
        self.tasks.append(drain)
        await asyncio.sleep(0)
        self.assertFalse(drain.done())
        value.policy_release.set()
        await asyncio.wait_for(drain, 2)
        self.assertFalse(value.scope.pending)
        self.assertEqual((value.policy_calls, value.snapshots), (1, 0))
        with self.assertRaises(ValueError):
            await value.prepare_context()


if __name__ == '__main__':
    unittest.main()
