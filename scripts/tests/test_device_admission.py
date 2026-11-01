"""Offline, event-driven checks of device-startup reservation ownership."""
import ast
import asyncio
from contextlib import asynccontextmanager
import json
import os
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, Mock

SOURCE = Path(os.environ.get('FRAMERIZE_SIDECAR_TEST_SOURCE',
                            Path(__file__).parents[1] / 'camoufox_host.py'))
BUDGET = 64 * 1024**2


class DeviceAdmissionTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.headroom = BUDGET
        tree = ast.parse(SOURCE.read_text())
        classes = {n.name: n for n in tree.body if isinstance(n, ast.ClassDef)}
        host = classes['Host']
        host.body = [n for n in host.body if isinstance(n, ast.AsyncFunctionDef)
                     and n.name in ('_allocation', '_prepare_device')]
        namespace = dict(asyncio=asyncio, asynccontextmanager=asynccontextmanager,
                         json=json, emit=Mock(), trace=Mock(), WARM_JS='offline-warmup',
                         LOGIN_URL='https://fixture.invalid',
                         memory_snapshot=lambda: {'allocation_headroom_bytes': self.headroom})
        selected = ast.Module(body=[classes['MemoryPressureError'], host], type_ignores=[])
        exec(compile(selected, str(SOURCE), 'exec'), namespace)
        self.host = namespace['Host']()
        self.host._allocation_lock = asyncio.Lock()
        self.host._reserved_bytes = 0
        self.pressure = namespace['MemoryPressureError']

    def device(self, existing=False, warm=False):
        page = SimpleNamespace(is_closed=Mock(return_value=False),
                               goto=AsyncMock(), evaluate=AsyncMock())
        ctx = SimpleNamespace(add_init_script=AsyncMock(), add_cookies=AsyncMock(),
                              new_page=AsyncMock(return_value=page), close=AsyncMock())
        acc = SimpleNamespace(session_cookie=None,
                              browser=SimpleNamespace(new_context=AsyncMock(return_value=ctx)))
        dev = SimpleNamespace(id='fixture-device', ctx=ctx if existing else None,
                              page=page if existing else None, warm=warm)
        return acc, dev, page, ctx

    async def block_warmup(self, acc, dev, page):
        entered, finish = asyncio.Event(), asyncio.Event()
        async def warm(script):
            self.assertEqual(script, 'offline-warmup')
            entered.set()
            await finish.wait()
        page.evaluate.side_effect = warm
        task = asyncio.create_task(self.host._prepare_device(acc, 'fixture-account', dev))
        async def cleanup():
            finish.set()
            try:
                await task
            except asyncio.CancelledError:
                pass
        self.addAsyncCleanup(cleanup)
        await asyncio.wait_for(entered.wait(), 1)
        return task, finish

    async def test_new_page_reservation_covers_warmup(self):
        acc, dev, page, _ = self.device()
        task, finish = await self.block_warmup(acc, dev, page)
        self.assertEqual(self.host._reserved_bytes, BUDGET)
        finish.set()
        await task
        self.assertEqual(self.host._reserved_bytes, 0)
        self.assertTrue(dev.warm)

    async def test_retry_warmup_reserves_without_creating_another_page(self):
        acc, dev, page, _ = self.device(existing=True)
        await self.block_warmup(acc, dev, page)
        self.assertEqual(self.host._reserved_bytes, BUDGET)
        acc.browser.new_context.assert_not_awaited()

    async def test_overlapping_startup_is_deferred_before_context_creation(self):
        acc, dev, page, _ = self.device()
        await self.block_warmup(acc, dev, page)
        other, other_dev, _, _ = self.device()
        with self.assertRaises(self.pressure):
            await self.host._prepare_device(other, 'other-account', other_dev)
        other.browser.new_context.assert_not_awaited()

    async def test_warm_page_remains_available_below_reserve(self):
        acc, dev, page, _ = self.device(existing=True, warm=True)
        self.headroom = -1
        self.assertIs(await self.host._prepare_device(acc, 'fixture-account', dev), dev)
        acc.browser.new_context.assert_not_awaited()
        page.evaluate.assert_not_awaited()
        self.assertEqual(self.host._reserved_bytes, 0)

    async def test_warmup_error_releases_reservation_and_preserves_retry(self):
        acc, dev, page, ctx = self.device()
        page.evaluate.side_effect = RuntimeError('offline failure')
        await self.host._prepare_device(acc, 'fixture-account', dev)
        self.assertEqual(self.host._reserved_bytes, 0)
        self.assertFalse(dev.warm)
        self.assertIs(dev.page, page)
        self.assertIs(dev.ctx, ctx)

    async def test_cancellation_releases_reservation_and_keeps_owned_page(self):
        acc, dev, page, ctx = self.device()
        task, _ = await self.block_warmup(acc, dev, page)
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertEqual(self.host._reserved_bytes, 0)
        self.assertFalse(dev.warm)
        self.assertIs(dev.page, page)
        self.assertIs(dev.ctx, ctx)


if __name__ == '__main__':
    unittest.main()
