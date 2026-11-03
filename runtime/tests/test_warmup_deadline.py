"""A stalled SDK warmup must close its unused context before returning."""
import asyncio
import unittest
from unittest.mock import AsyncMock, patch
import test_device_context_creation as creation

class WarmupDeadlineTests(unittest.IsolatedAsyncioTestCase):
    setUp = creation.DeviceContextCreationTests.setUp
    finish = creation.DeviceContextCreationTests.finish
    account = creation.DeviceContextCreationTests.account
    prepare = creation.DeviceContextCreationTests.prepare
    command = creation.DeviceContextCreationTests.command
    start = creation.DeviceContextCreationTests.start
    block = creation.DeviceContextCreationTests.block
    assert_idle = creation.DeviceContextCreationTests.assert_idle

    def held_warmup(self):
        acc = self.account()
        context = acc.browser.next_context
        entered, release = asyncio.Event(), asyncio.Event()
        async def evaluate(_):
            entered.set()
            await release.wait()
            if context.closed:
                raise RuntimeError('fixture context closed')
            return 'fixture-token'
        close = context.close
        async def close_and_release():
            await close()
            release.set()
        context.page.evaluate = AsyncMock(side_effect=evaluate)
        context.close = AsyncMock(side_effect=close_and_release)
        return acc, context, entered, release

    async def test_timeout_closes_context_and_releases_raw_call(self):
        acc, context, entered, release = self.held_warmup()
        with patch.object(self.module, 'SDK_WARMUP_TIMEOUT_S', .02, create=True):
            request = self.command()
            await asyncio.wait_for(entered.wait(), 1)
            try:
                with self.assertRaisesRegex(RuntimeError, 'SDK warmup.*timed out.*request was not sent'):
                    await asyncio.wait_for(asyncio.shield(request), .3)
                self.assertTrue(context.closed)
                self.assertIsNone(acc.dev_objs['device-a'].ctx)
                self.assertFalse(acc.dev_objs['device-a'].warm)
                self.assert_idle(acc)
            finally:
                release.set()
                await asyncio.gather(request, return_exceptions=True)

    async def test_failed_warmup_cannot_return_an_unprepared_device(self):
        acc = self.account()
        context = acc.browser.next_context
        context.page.evaluate.side_effect = RuntimeError('fixture SDK download failed')
        with self.assertRaisesRegex(RuntimeError, 'fixture SDK download failed'):
            await self.command()
        self.assertTrue(context.closed)
        self.assertIsNone(acc.dev_objs['device-a'].ctx)
        self.assert_idle(acc)

    async def test_cancelled_warmup_closes_its_context(self):
        acc, context, entered, release = self.held_warmup()
        request = self.command()
        await asyncio.wait_for(entered.wait(), 1)
        request.cancel('client left during warmup')
        try:
            with self.assertRaises(asyncio.CancelledError):
                await asyncio.wait_for(asyncio.shield(request), .3)
            self.assertTrue(context.closed)
            self.assertIsNone(acc.dev_objs['device-a'].ctx)
            self.assert_idle(acc)
        finally:
            release.set()
            await asyncio.gather(request, return_exceptions=True)

    async def test_failed_cleanup_keeps_the_device_fenced(self):
        acc = self.account()
        context = acc.browser.next_context
        context.page.evaluate.side_effect = RuntimeError('fixture SDK failed')
        context.disposal.failure = RuntimeError('fixture close failed')
        with self.assertRaisesRegex(RuntimeError, 'fixture close failed'):
            await self.command()
        self.assertIs(acc.dev_objs['device-a'].ctx, context)
        self.assertEqual(acc.lifecycle.phase, 'unconfirmed')
        self.assertTrue(acc.closing)
        self.assertFalse(context.closed)
