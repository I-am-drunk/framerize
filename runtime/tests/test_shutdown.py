"""Shutdown through the real host, with synthetic browsers and private history."""
import asyncio
import io
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_host import app
from framerize_accounts.journal import RateJournal
from framerize_host.browser import driver
from test_lifecycle import FakeBrowser


class ShutdownTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.journal = RateJournal(directory.name)
        self.addCleanup(self.journal.close)
        for override in (patch.object(app, 'ROOT', directory.name),
                         patch.object(driver, 'verify_close_protocol', lambda: None)):
            override.start()
            self.addCleanup(override.stop)
        self.host = app.Host(self.journal)

    def resident(self, key):
        life = self.host._lifecycle(key)
        browser = FakeBrowser()
        life.browser, life.phase = browser, 'resident'
        life.close_receipt = app.BrowserCloseReceipt(browser)
        device = life.rates.ensure_devices(1)[0]
        acc = app.Account(None, None, None, life)
        acc.browser = browser
        self.host.accounts[key] = acc
        self.host._note_request(acc, device)
        return life, browser

    async def test_cancelled_shutdown_waiter_still_joins_rpc_and_saves_close(self):
        life, browser = self.resident('account')
        before = self.journal.load('account')['state']['requests']
        entered, release = asyncio.Event(), asyncio.Event()
        async def raw():
            entered.set()
            await release.wait()
        waiter = asyncio.create_task(self.host._rpc('account', raw))
        await asyncio.wait_for(entered.wait(), 1)
        waiter.cancel()
        await asyncio.gather(waiter, return_exceptions=True)
        closing = asyncio.create_task(self.host.close())
        await asyncio.sleep(0)
        closing.cancel()
        await asyncio.gather(closing, return_exceptions=True)
        try:
            self.assertFalse(self.host._close_task.done())
            self.assertEqual(browser.calls, 0)
            self.assertFalse(self.journal.load('account')['state']['clean'])
            with self.assertRaisesRegex(RuntimeError, 'shutting down'):
                await self.host.handle({'cmd': 'browser_status', 'account': 'other'})
        finally:
            release.set()
            await asyncio.wait_for(self.host.close(), 1)
        self.assertEqual(browser.calls, 1)
        self.assertFalse(life.activity.pending)
        saved = self.journal.load('account')['state']
        self.assertTrue(saved['clean'])
        self.assertEqual(saved['requests'], before)

    async def test_close_failure_stays_dirty_but_other_accounts_close(self):
        failed, broken_browser = self.resident('broken')
        healthy, browser = self.resident('healthy')
        broken_browser.failure = True
        driver_port = SimpleNamespace(stop=AsyncMock())
        self.host._playwright = driver_port
        for _ in range(2):
            with self.assertRaisesRegex(RuntimeError, 'fixture close acknowledgement lost'):
                await asyncio.wait_for(self.host.close(), 1)
        self.assertEqual(failed.phase, 'unconfirmed')
        self.assertFalse(self.journal.load('broken')['state']['clean'])
        self.assertTrue(self.journal.load('healthy')['state']['clean'])
        self.assertFalse(browser.connected)
        self.assertEqual(broken_browser.calls, 1)
        driver_port.stop.assert_awaited_once_with()

    async def test_shutdown_keeps_reader_alive_for_a_delayed_close_ack(self):
        life, browser = self.resident('account')
        browser.release.clear()
        port = SimpleNamespace(stop=AsyncMock())
        self.host._playwright = port
        with patch.object(app, 'IDLE_CLOSE_TIMEOUT_S', 0.01), \
             patch.object(app, 'SHUTDOWN_CLOSE_TIMEOUT_S', 1):
            closing = asyncio.create_task(self.host.close())
            await asyncio.wait_for(browser.called.wait(), 1)
            await asyncio.sleep(0.03)
            self.assertFalse(closing.done())
            port.stop.assert_not_awaited()
            browser.release.set()
            await asyncio.wait_for(closing, 1)
        self.assertEqual(browser.calls, 1)
        self.assertEqual(life.phase, 'absent')
        self.assertTrue(self.journal.load('account')['state']['clean'])
        port.stop.assert_awaited_once_with()

    async def test_idle_browsers_close_concurrently_before_driver_stop(self):
        first, browser = self.resident('first')
        second, other = self.resident('second')
        browser.release.clear()
        other.release.clear()
        port = SimpleNamespace(stop=AsyncMock())
        self.host._playwright = port
        closing = asyncio.create_task(self.host.close())
        try:
            await asyncio.wait_for(asyncio.gather(
                browser.called.wait(), other.called.wait()), 1)
            port.stop.assert_not_awaited()
        finally:
            browser.release.set()
            other.release.set()
            await asyncio.wait_for(closing, 1)
        self.assertEqual((first.phase, second.phase), ('absent', 'absent'))
        port.stop.assert_awaited_once_with()

    async def test_timed_out_close_stops_driver_and_releases_pending_rpc(self):
        life, browser = self.resident('account')
        browser.release.clear()
        async def stop_driver():
            browser.failure = True
            browser.release.set()
            await asyncio.gather(*life.activity.pending, return_exceptions=True)
        port = SimpleNamespace(stop=AsyncMock(side_effect=stop_driver))
        self.host._playwright = port
        with patch.object(app, 'SHUTDOWN_CLOSE_TIMEOUT_S', 0.03):
            closing = asyncio.create_task(self.host.close())
            await asyncio.wait_for(browser.called.wait(), 1)
            browser.disconnect()
            with self.assertRaisesRegex(RuntimeError, 'disconnected before Playwright acknowledged'):
                await asyncio.wait_for(closing, 1)
        self.assertFalse(life.activity.pending)
        self.assertEqual(life.phase, 'unconfirmed')
        self.assertFalse(self.journal.load('account')['state']['clean'])
        port.stop.assert_awaited_once_with()

    async def test_shutdown_preserves_unexpected_close_error_without_credentials(self):
        self.resident('account')
        failure = RuntimeError('history write failed: disk full; Authorization: Bearer private-fixture')
        with patch.object(self.host, '_close_account', AsyncMock(side_effect=failure)):
            with self.assertRaises(RuntimeError) as caught:
                await self.host.close()
        self.assertIn('history write failed: disk full', str(caught.exception))
        self.assertNotIn('private-fixture', str(caught.exception))
        self.assertFalse(self.journal.load('account')['state']['clean'])

    async def test_background_loops_stop_before_driver_and_close_runs_once(self):
        _, browser = self.resident('account')
        stopped = asyncio.Event()
        async def loop():
            try:
                await asyncio.Future()
            finally:
                stopped.set()
        task = self.host._track_task(self.host.accounts['account'], loop())
        await asyncio.sleep(0)
        port = SimpleNamespace(stop=AsyncMock())
        self.host._playwright = port
        await asyncio.gather(self.host.close(), self.host.close())
        self.assertTrue(stopped.is_set())
        self.assertTrue(task.cancelled())
        self.assertEqual(browser.calls, 1)
        port.stop.assert_awaited_once_with()

    async def test_protocol_shutdown_drains_but_eof_cancels_command_followup(self):
        for graceful in (True, False):
            with self.subTest(graceful=graceful):
                host = app.Host()
                entered, release = asyncio.Event(), asyncio.Event()
                completed = []
                async def serve(*_):
                    entered.set()
                    await release.wait()
                    completed.append(True)
                host._serve = serve
                writer = SimpleNamespace(send=AsyncMock(), close=AsyncMock())
                lines = '{"id": 1, "cmd": "fixture"}\n'
                if graceful:
                    lines += '{"cmd": "shutdown"}\n'
                with patch.object(app.sys, 'stdin', io.StringIO(lines)), \
                     patch.object(app, 'AsyncReplyWriter', return_value=writer):
                    task = asyncio.create_task(host.loop())
                    await asyncio.wait_for(entered.wait(), 1)
                    if graceful:
                        await asyncio.sleep(0.02)
                        self.assertFalse(task.done())
                        writer.close.assert_not_awaited()
                        release.set()
                    await asyncio.wait_for(task, 1)
                self.assertEqual(completed, [True] if graceful else [])
                writer.close.assert_awaited_once_with()
