"""Exercise real account ownership with inert browser ports."""
import asyncio
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_host import app
from framerize_host.browser import driver
from test_lifecycle import FakeBrowser


class ActivityTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        override = patch.multiple(app, ROOT=directory.name, emit=lambda *a, **k: None)
        override.start()
        self.addCleanup(override.stop)
        protocol = patch.object(driver, 'verify_close_protocol', lambda: None)
        protocol.start()
        self.addCleanup(protocol.stop)
        self.host = app.Host()
        self.life = app.AccountLifecycle()
        self.browser = FakeBrowser()
        self.life.browser, self.life.phase = self.browser, 'resident'
        self.life.close_receipt = app.BrowserCloseReceipt(self.browser)
        self.page = SimpleNamespace()
        self.acc = app.Account(None, None, self.page, self.life)
        self.acc.browser = self.browser
        self.host.accounts['a'], self.host.lifecycles['a'] = self.acc, self.life
        self.entered, self.release = asyncio.Event(), asyncio.Event()
        self.calls = []

    async def held(self):
        self.entered.set()
        await self.release.wait()
        self.calls.append('completed')
        return 'done'

    async def asyncTearDown(self):
        self.release.set()
        await asyncio.gather(*self.life.activity.pending, return_exceptions=True)
        await self.host.close()

    async def test_cancelled_waiter_keeps_rpc_owned_and_prevents_early_close(self):
        waiter = asyncio.create_task(self.host._rpc('a', self.held))
        await asyncio.wait_for(self.entered.wait(), 1)
        waiter.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await waiter
        self.assertEqual(len(self.life.activity.pending), 1)
        self.assertFalse(await self.host._close_account('a'))
        self.assertTrue(self.browser.connected)
        self.release.set()
        await asyncio.gather(*self.life.activity.pending)
        self.assertEqual(self.calls, ['completed'])
        self.assertTrue(await self.host._close_account('a'))

    async def test_cancelled_job_retains_poll_owner_after_abort(self):
        async def evaluate(script, args):
            if script == app.AI_POLL_JS:
                return await self.held()
            if script == app.AI_CANCEL_JS:
                self.calls.append('abort')
        self.page.evaluate = evaluate
        waiter = asyncio.create_task(self.host._run_job(
            self.page, '/fixture', {}, 'token', 'bearer', 'job', 'device', 30))
        await asyncio.wait_for(self.entered.wait(), 1)
        waiter.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await waiter
        self.assertEqual(self.calls, ['abort'])
        self.assertEqual(len(self.life.activity.pending), 1)
        self.assertFalse(await self.host._close_account('a'))

    async def test_unsupported_native_commands_never_start_browser(self):
        self.host._launch = AsyncMock(side_effect=AssertionError('unexpected launch'))
        for command in ('native_device_open', 'resident_authorization_once',
                        'resident_authorization_bootstrap', 'browser_baseline_once',
                        'prepare_inference_authorization'):
            with self.subTest(command=command), self.assertRaisesRegex(RuntimeError, 'unknown cmd'):
                await self.host.handle({'cmd': command, 'account': 'b'})
        self.host._launch.assert_not_called()

    async def test_cancelled_abort_waiter_retains_cleanup_rpc_ownership(self):
        polling = asyncio.Event()
        async def evaluate(script, args):
            if script == app.AI_POLL_JS:
                polling.set()
                return None
            if script == app.AI_CANCEL_JS:
                return await self.held()
        self.page.evaluate = evaluate
        waiter = asyncio.create_task(self.host._run_job(
            self.page, '/fixture', {}, 'token', 'bearer', 'job', None, 30))
        await asyncio.wait_for(polling.wait(), 1)
        waiter.cancel()
        await asyncio.wait_for(self.entered.wait(), 1)
        self.assertFalse(waiter.done())
        waiter.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await waiter
        self.assertEqual(len(self.life.activity.pending), 1)
        self.assertFalse(await self.host._close_account('a'))
        self.release.set()
        await asyncio.gather(*self.life.activity.pending)
        self.assertTrue(await self.host._close_account('a'))

    async def test_command_cancellation_stops_followup_work(self):
        async def command(request):
            await self.host._rpc('a', self.held)
            self.fail('cancelled command resumed')
        self.host._handle = command
        waiter = asyncio.create_task(self.host.handle({'cmd': 'eval', 'account': 'a'}))
        await asyncio.wait_for(self.entered.wait(), 1)
        waiter.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await waiter
        self.assertEqual(self.life.active_commands, 0)
        self.assertEqual(self.acc.active_requests, 0)
        self.release.set()
        await asyncio.gather(*self.life.activity.pending)
        self.assertEqual(self.calls, ['completed'])
        self.assertFalse(self.life.activity.activities)

    async def test_timeout_keeps_actual_rpc_owned(self):
        with self.assertRaises(asyncio.TimeoutError):
            await asyncio.wait_for(self.host._rpc('a', self.held), 0.02)
        self.assertTrue(self.entered.is_set())
        self.assertEqual(len(self.life.activity.pending), 1)
        self.assertFalse(next(iter(self.life.activity.pending)).cancelled())

    async def test_late_browser_launch_keeps_its_receipt_until_confirmed_close(self):
        browser = FakeBrowser()
        async def launch(*args, **kwargs):
            await self.held()
            return browser
        self.host._browser_runtime = (launch, None)
        self.host._playwright = SimpleNamespace(stop=AsyncMock())
        self.host._initialize_account = AsyncMock(side_effect=AssertionError('cancelled startup'))
        with patch.object(app, 'memory_snapshot', return_value={}):
            waiter = asyncio.create_task(self.host._account('late'))
            await asyncio.wait_for(self.entered.wait(), 1)
            waiter.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await waiter
            life = self.host.lifecycles['late']
            self.assertEqual(life.phase, 'unconfirmed')
            self.assertFalse(await self.host._close_account('late'))
            self.release.set()
            await asyncio.gather(*life.activity.pending)
        self.assertIs(life.browser, browser)
        self.assertIs(life.close_receipt.browser, browser)
        self.host._initialize_account.assert_not_awaited()
        self.assertTrue(await self.host._close_account('late'))
        self.assertFalse(browser.connected)
        self.assertEqual(browser.calls, 1)
        self.assertEqual(life.phase, 'absent')

    async def test_failed_rpc_releases_activity(self):
        with self.assertRaisesRegex(RuntimeError, 'fixture'):
            await self.host._rpc('a', AsyncMock(side_effect=RuntimeError('fixture')))
        self.assertFalse(self.life.activity.pending)
        self.assertFalse(self.life.activity.activities)

    async def test_background_task_owns_activity_until_completion(self):
        task = self.host._track_task(self.acc, self.held())
        self.assertTrue(self.life.activity.activities)
        await asyncio.wait_for(self.entered.wait(), 1)
        self.release.set()
        await task
        self.assertFalse(self.life.activity.activities)
        self.assertFalse(self.acc.tasks)

    async def test_shared_driver_start_survives_one_cancelled_waiter(self):
        driver = SimpleNamespace(stop=AsyncMock())
        async def start():
            await self.held()
            return driver
        factory = lambda: SimpleNamespace(start=start)
        first = asyncio.create_task(self.host._start_driver(factory))
        await asyncio.wait_for(self.entered.wait(), 1)
        second = asyncio.create_task(self.host._start_driver(factory))
        first.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await first
        self.release.set()
        await second
        self.assertIs(self.host._playwright, driver)
        self.assertEqual(self.calls, ['completed'])
