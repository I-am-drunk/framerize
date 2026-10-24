"""Inert full-jar settling, strict identity and retained-RPC checks."""
import asyncio
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('_settling_readiness',
    Path(__file__).with_name('browser_comparison_readiness_tests.py'))
R = importlib.util.module_from_spec(spec)
spec.loader.exec_module(R)
C = R.C

class SettlingTests(unittest.IsolatedAsyncioTestCase):
    asyncSetUp = R.BrowserComparisonReadinessTests.asyncSetUp
    asyncTearDown = R.BrowserComparisonReadinessTests.asyncTearDown
    fixture = R.BrowserComparisonReadinessTests.fixture

    def timing(self, limit=.3):
        return patch.multiple(C, COMPARISON_SETTLE_SECONDS=.03,
            COMPARISON_SETTLE_POLL=.005, COMPARISON_SETTLE_LIMIT=limit)

    async def test_startup_cookie_changes_settle_before_binding(self):
        value = self.fixture()
        row = {'name':'context', 'value':'first'}
        value.context.rows.append(row)
        reads = []
        def changed():
            row['value'] = 'second' if len(reads) else 'first'
            reads.append(row['value'])
        value.context.on_cookies = changed
        with self.timing():
            reply = await value.prepare_context()
        self.assertIn('context=second', reply['cookie_header'])
        self.assertGreater(reads.count('second'), 1)
        self.assertEqual((value.mints, value.fetches), (0, 0))
        value.context.on_cookies = None
        row['value'] = 'later'
        self.assertFalse((await value.baseline())['accepted'])
        self.assertEqual((value.mints, value.fetches), (0, 0))

    async def test_continuous_cookie_changes_exhaust_the_bound(self):
        value = self.fixture()
        row = {'name':'context', 'value':'0'}
        value.context.rows.append(row)
        def changed(): row['value'] = str(int(row['value']) + 1)
        value.context.on_cookies = changed
        with self.timing(limit=.06):
            with self.assertRaises(RuntimeError):
                await value.prepare_context()
        self.assertIsNone(value.scope.comparison)
        self.assertEqual(value.events[-1]['stage'], 'context_settle_timeout')
        self.assertEqual((value.mints, value.fetches), (0, 0))

    async def test_page_or_device_drift_refuses_even_if_cookies_stabilize(self):
        for kind in ('page','device'):
            value = self.fixture()
            reads = 0
            def drift():
                if kind == 'page': value.page.url += '?changed'
                else:
                    value.page.device = 'new-device'
                    value.context.rows[1]['value'] = 'new-device'
            def changed():
                nonlocal reads
                reads += 1
                if reads == 1: asyncio.get_running_loop().call_soon(drift)
            value.context.on_cookies = changed
            with self.timing():
                with self.assertRaises(RuntimeError): await value.prepare_context()
            self.assertIsNone(value.scope.comparison)
            # The existing snapshot guard can detect the change before the
            # settling loop receives that sample. Both paths must refuse.
            self.assertIn(value.events[-1]['stage'],
                          ('snapshot_validate', 'context_settle_drift'))
            self.assertEqual((value.mints, value.fetches), (0, 0))

    async def blocked_snapshot(self, cancel):
        value = self.fixture()
        entered, release = asyncio.Event(), asyncio.Event()
        original = value.context.cookies
        async def blocked(url):
            entered.set()
            await release.wait()
            return await original(url)
        value.context.cookies = blocked
        try:
            with self.timing(limit=.04):
                task = asyncio.create_task(value.prepare_context())
                self.tasks.append(task)
                await entered.wait()
                if cancel: task.cancel()
                with self.assertRaises(asyncio.CancelledError if cancel else RuntimeError):
                    await task
            self.assertTrue(value.scope.pending)
            self.assertIsNone(value.scope.comparison)
        finally:
            release.set()
            await asyncio.wait_for(value.scope.drain(), 2)
        self.assertEqual((value.mints, value.fetches), (0, 0))
        self.assertIsNone(value.scope.comparison)

    async def test_timeout_retains_the_pending_snapshot_rpc(self):
        await self.blocked_snapshot(False)

    async def test_cancellation_retains_rpc_and_never_binds_late_result(self):
        await self.blocked_snapshot(True)

if __name__ == '__main__':
    unittest.main()
