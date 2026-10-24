"""Inert device-readiness fixtures. UNRUN by the source implementation owner."""
import asyncio
import importlib.util
import json
from pathlib import Path
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "_readiness_diagnostics", Path(__file__).with_name("browser_comparison_diagnostics_tests.py"))
D = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(D)
C = D.C
SECRET = "private-readiness-sentinel"


class ControlledHandle:
    def __init__(self, owner):
        self.owner = owner

    async def dispose(self):
        owner = self.owner
        owner.calls.append(("dispose_readiness",))
        owner.dispose_started.set()
        try:
            await owner.dispose_release.wait()
        except asyncio.CancelledError:
            owner.dispose_cancelled = True
            raise
        if owner.dispose_error:
            raise RuntimeError(SECRET)
        owner.disposed = True


class ReadinessHarness(D.DiagnosticHarness):
    def __init__(self):
        super().__init__()
        self.ready_started, self.ready_release = asyncio.Event(), asyncio.Event()
        self.dispose_started, self.dispose_release = asyncio.Event(), asyncio.Event()
        self.ready_release.set()
        self.dispose_release.set()
        self.ready_error = self.dispose_error = self.dispose_cancelled = self.disposed = False
        self.after_ready, self.snapshots = None, 0

    async def wait_for_function(self, script, *, arg, timeout, polling):
        assert arg == {'project': D.FIXTURE.OLD.PROJECT}
        self.calls.append(("wait_for_function", script, timeout, polling, arg))
        self.ready_started.set()
        await self.ready_release.wait()
        if self.ready_error:
            raise TimeoutError(SECRET)
        assert type(self.page.device) is str and self.page.device
        if self.after_ready is not None:
            self.after_ready()
        return ControlledHandle(self)

    async def evaluate(self, script, args=None):
        if script == "mw:" + C.COMPARISON_SNAPSHOT:
            self.snapshots += 1
            assert self.disposed, "snapshot must follow handle disposal"
        return await super().evaluate(script, args)


class BrowserComparisonReadinessTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.values, self.tasks = [], []

    def fixture(self):
        value = ReadinessHarness()
        self.values.append(value)
        return value

    async def asyncTearDown(self):
        for value in self.values:
            value.page.device = D.FIXTURE.DEVICE
            value.ready_release.set()
            value.dispose_release.set()
            value.page.sdk_release.set()
            value.fetch_release.set()
        for task in self.tasks:
            if not task.done():
                task.cancel()
        await asyncio.gather(*self.tasks, return_exceptions=True)
        for value in self.values:
            await asyncio.wait_for(value.scope.drain(), 2)

    def assert_pre_snapshot_failure(self, value):
        self.assertEqual((value.snapshots, value.mints, value.fetches), (0, 0, 0))
        self.assertEqual(value.scope.comparison_phase, "failed")
        self.assertIsNone(value.scope.comparison)
        self.assertEqual(value.events, [{"event": "browser.comparison.command",
            "outcome": "failed", "stage": "editor_readiness"}])
        self.assertNotIn(SECRET, json.dumps(value.events))

    async def test_delayed_readiness_disposes_before_snapshot_and_keeps_one_shot_pair(self):
        value = self.fixture()
        value.page.device = None
        value.ready_release.clear()
        value.dispose_release.clear()
        task = asyncio.create_task(value.prepare_context())
        self.tasks.append(task)
        await asyncio.wait_for(value.ready_started.wait(), 2)
        self.assertEqual(C.comparison_evidence(value.scope), {"stage": "editor_readiness"})
        self.assertEqual((value.snapshots, value.mints, value.fetches), (0, 0, 0))
        value.page.device = D.FIXTURE.DEVICE
        value.ready_release.set()
        await asyncio.wait_for(value.dispose_started.wait(), 2)
        self.assertFalse(task.done())
        self.assertEqual(value.snapshots, 0)
        value.dispose_release.set()
        context = await asyncio.wait_for(task, 2)
        self.assertTrue(value.disposed)
        self.assertEqual(value.snapshots, 1)
        self.assertNotIn("token", context)
        ready = [call for call in value.calls if call[0] == "wait_for_function"]
        self.assertEqual(len(ready), 1)
        self.assertEqual(ready[0][2:], (10000, 100, {'project': D.FIXTURE.OLD.PROJECT}))
        self.assertTrue((await value.baseline())["accepted"])
        await value.native()
        self.assertEqual((value.mints, value.fetches), (2, 1))
        names = [call[0] for call in value.calls]
        for name in ("goto", "add_cookies", "wait_for_function", "dispose_readiness"):
            self.assertEqual(names.count(name), 1)

    async def test_readiness_timeout_never_snapshots_mints_or_retries(self):
        value = self.fixture()
        value.ready_error = True
        with self.assertRaisesRegex(RuntimeError, "^browser comparison failed$") as caught:
            await value.prepare_context()
        self.assertIsNone(caught.exception.__cause__)
        self.assertTrue(caught.exception.__suppress_context__)
        self.assert_pre_snapshot_failure(value)
        self.assertFalse(value.dispose_started.is_set())
        with self.assertRaises(ValueError):
            await value.prepare_context()
        self.assertEqual(sum(call[0] == "wait_for_function" for call in value.calls), 1)

    async def test_initial_routing_finishes_before_the_full_url_is_bound(self):
        value = self.fixture()
        value.ready_release.clear()
        task = asyncio.create_task(value.prepare_context())
        self.tasks.append(task)
        await asyncio.wait_for(value.ready_started.wait(), 2)
        self.assertIsNone(value.scope.comparison)
        self.assertEqual((value.snapshots, value.mints, value.fetches), (0, 0, 0))
        url = 'https://framer.com/projects/Fixture--' + D.FIXTURE.OLD.PROJECT + '?node=selected'
        value.page.url = url
        value.ready_release.set()
        result = await asyncio.wait_for(task, 2)
        self.assertEqual(result['page_url'], url)
        self.assertEqual(value.scope.comparison['url'], url)
        self.assertEqual((value.snapshots, value.mints, value.fetches), (1, 0, 0))

    async def test_cancelled_readiness_retains_rpc_until_late_handle_is_disposed(self):
        value = self.fixture()
        value.ready_release.clear()
        value.dispose_release.clear()
        task = asyncio.create_task(value.prepare_context())
        self.tasks.append(task)
        await asyncio.wait_for(value.ready_started.wait(), 2)
        owned = value.scope.submitted[-1]
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertFalse(owned.done())
        self.assertEqual(value.scope.comparison_phase, "failed")
        value.scope.stopped.set()
        value.ready_release.set()
        await asyncio.wait_for(value.dispose_started.wait(), 2)
        self.assertIn(owned, value.scope.pending)
        value.dispose_release.set()
        await asyncio.wait_for(value.scope.drain(), 2)
        self.assertTrue(value.disposed)
        self.assertFalse(value.scope.pending)
        self.assertEqual((value.snapshots, value.mints, value.fetches), (0, 0, 0))
        with self.assertRaises(ValueError):
            await value.prepare_context()

    async def test_cancellation_during_disposal_keeps_handle_owned(self):
        value = self.fixture()
        value.dispose_release.clear()
        task = asyncio.create_task(value.prepare_context())
        self.tasks.append(task)
        await asyncio.wait_for(value.dispose_started.wait(), 2)
        owned = value.scope.submitted[-1]
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertFalse(owned.done())
        self.assertFalse(value.dispose_cancelled)
        value.dispose_release.set()
        await asyncio.wait_for(value.scope.drain(), 2)
        self.assertTrue(value.disposed)
        self.assertEqual(value.scope.comparison_phase, "failed")
        self.assertEqual((value.snapshots, value.mints, value.fetches), (0, 0, 0))

    async def test_outer_deadline_retains_unresolved_readiness_rpc(self):
        value = self.fixture()
        value.ready_release.clear()
        with patch.object(C, "COMPARISON_READY_SECONDS", 0.01), \
                patch.object(C, "COMPARISON_DISPOSE_SECONDS", 0.01):
            with self.assertRaisesRegex(RuntimeError, "^browser comparison failed$"):
                await value.prepare_context()
        self.assert_pre_snapshot_failure(value)
        owned = value.scope.submitted[-1]
        self.assertFalse(owned.done())
        value.ready_release.set()
        await asyncio.wait_for(value.scope.drain(), 2)
        self.assertTrue(value.disposed)
        self.assertFalse(value.scope.pending)
        self.assertEqual(value.snapshots, 0)

    async def test_disposal_timeout_keeps_cleanup_owned_and_refuses_snapshot(self):
        value = self.fixture()
        value.dispose_release.clear()
        with patch.object(C, "COMPARISON_DISPOSE_SECONDS", 0.01):
            with self.assertRaisesRegex(RuntimeError, "^browser comparison failed$"):
                await value.prepare_context()
        self.assert_pre_snapshot_failure(value)
        self.assertFalse(value.dispose_cancelled)
        self.assertFalse(value.disposed)
        self.assertIn(value.scope.submitted[-1], value.scope.pending)
        value.dispose_release.set()
        await asyncio.wait_for(value.scope.drain(), 2)
        self.assertTrue(value.disposed)

    async def test_disposal_error_is_private_and_prevents_snapshot(self):
        value = self.fixture()
        value.dispose_error = True
        with self.assertRaisesRegex(RuntimeError, "^browser comparison failed$"):
            await value.prepare_context()
        self.assert_pre_snapshot_failure(value)
        self.assertFalse(value.disposed)

    async def test_final_snapshot_still_rejects_device_cookie_login_page_and_url_drift(self):
        cases = (
            ("device_valid", lambda v: setattr(v.page, "device", None)),
            ("device_valid", lambda v: setattr(v.page, "device", "x" * 257)),
            ("device_cookie_match", lambda v: v.context.rows[1].update(value=SECRET)),
            ("login_match", lambda v: v.context.rows[0].update(value=SECRET)),
            ("page_match", lambda v: setattr(v.page, "url", "https://framer.com/projects/other")),
            ("url_unchanged", lambda v: setattr(v.context, "changed_during_cookies",
                                               "https://framer.com/projects/other")),
        )
        for check, change in cases:
            with self.subTest(check=check):
                value = self.fixture()
                value.after_ready = lambda value=value, change=change: change(value)
                with self.assertRaisesRegex(RuntimeError, "^browser comparison failed$"):
                    await value.prepare_context()
                self.assertTrue(value.disposed)
                self.assertEqual((value.snapshots, value.mints, value.fetches), (1, 0, 0))
                self.assertEqual(value.scope.comparison_phase, "failed")
                self.assertEqual(value.events[0]["stage"], "snapshot_validate")
                self.assertIs(value.events[0][check], False)
                self.assertNotIn(SECRET, json.dumps(value.events))
                seeds = [call[1] for call in value.calls if call[0] == "add_cookies"]
                self.assertEqual([[row["name"] for row in seed] for seed in seeds], [["session"]])


if __name__ == "__main__":
    unittest.main()
