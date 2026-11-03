"""Slow runtime preparation must not block host status or lose its owner."""
import asyncio
import tempfile
import threading
import unittest
from unittest.mock import patch
from test_lifecycle import app


class RuntimePreparationTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.entered, self.release = threading.Event(), threading.Event()
        self.calls, self.released = 0, False
        self.addCleanup(self.release.set)
        def prepare():
            self.calls += 1
            self.entered.set()
            self.released = self.release.wait(2)
            raise RuntimeError('fixture preparation ended')
        overrides = patch.multiple(app, ROOT=directory.name,
            browser_runtime=lambda: (None, None),
            prepared_memory_browser=prepare, prepared_memory_addon=lambda: None)
        overrides.start()
        self.addCleanup(overrides.stop)
        self.host = app.Host()

    async def wait_entered(self):
        for _ in range(1000):
            if self.entered.is_set():
                return
            await asyncio.sleep(.001)
        self.fail('runtime preparation did not start')

    async def test_status_answers_while_runtime_preparation_is_blocked(self):
        task = asyncio.create_task(self.host._launch('fixture'))
        await self.wait_entered()
        status = await self.host.handle(dict(cmd='browser_status', account='other'))
        self.release.set()
        with self.assertRaisesRegex(RuntimeError, 'fixture preparation ended'):
            await task
        self.assertEqual(status['state'], 'absent')
        self.assertTrue(self.released, 'preparation blocked the command loop')

    async def test_cancelled_waiter_does_not_start_duplicate_preparation(self):
        first = asyncio.create_task(self.host._launch('first'))
        await self.wait_entered()
        first.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await first
        second = asyncio.create_task(self.host._launch('second'))
        await asyncio.sleep(0)
        self.release.set()
        with self.assertRaisesRegex(RuntimeError, 'fixture preparation ended'):
            await second
        self.assertEqual(self.calls, 1)

    async def test_shutdown_joins_preparation_after_waiter_cancellation(self):
        waiter = asyncio.create_task(self.host._launch('fixture'))
        await self.wait_entered()
        waiter.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await waiter
        closing = asyncio.create_task(self.host.close())
        await asyncio.sleep(0)
        self.assertFalse(closing.done())
        self.release.set()
        await closing
        self.assertTrue(self.released)
        self.assertFalse(self.host.accounts)

    async def test_successful_preparation_is_shared_and_cached(self):
        with patch.object(app, 'prepared_memory_browser', return_value='fixture') as prepare:
            await asyncio.gather(self.host._prepare_runtime(), self.host._prepare_runtime())
            await self.host._prepare_runtime()
            self.assertEqual(prepare.call_count, 1)
        self.assertEqual(self.host._memory_browser, 'fixture')

    async def test_failed_preparation_can_retry_without_partial_runtime(self):
        self.release.set()
        with self.assertRaisesRegex(RuntimeError, 'fixture preparation ended'):
            await self.host._prepare_runtime()
        self.assertIsNone(self.host._browser_runtime)
        self.assertIsNone(self.host._memory_browser)
        with patch.object(app, 'prepared_memory_browser', return_value='recovered'):
            await self.host._prepare_runtime()
        self.assertEqual(self.host._memory_browser, 'recovered')

if __name__ == '__main__':
    unittest.main()
