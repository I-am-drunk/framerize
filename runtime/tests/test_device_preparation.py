"""Device preparation retains its dispatched RPCs through cancellation."""
import asyncio
import unittest
from unittest.mock import AsyncMock

import test_device_context_creation as creation
import test_page_acquisition as acquisition


class DevicePreparationTests(unittest.IsolatedAsyncioTestCase):
    setUp = creation.DeviceContextCreationTests.setUp
    finish = creation.DeviceContextCreationTests.finish
    account = creation.DeviceContextCreationTests.account
    prepare = creation.DeviceContextCreationTests.prepare
    command = creation.DeviceContextCreationTests.command
    start = creation.DeviceContextCreationTests.start
    block = creation.DeviceContextCreationTests.block
    assert_waiting = creation.DeviceContextCreationTests.assert_waiting
    assert_idle = creation.DeviceContextCreationTests.assert_idle

    def block_method(self, method, result=None):
        gate = self.block(acquisition.Call())
        async def invoke(*_args, **_kwargs):
            await gate()
            return result
        method.side_effect = invoke
        return gate

    async def test_cancelled_seed_retains_raw_rpc_and_device_lock(self):
        acc = self.account()
        context = acc.browser.next_context
        seeding = self.block_method(context.add_init_script)
        request = self.command()
        await asyncio.wait_for(seeding.entered.wait(), 1)
        request.cancel()
        await asyncio.wait_for(context.disposal.entered.wait(), 1)
        await self.assert_waiting(request)
        waiting = self.command()
        await self.assert_waiting(waiting)
        self.assertTrue(acc.dev_objs["device-a"].lock.locked())
        self.assertEqual(self.host._reserved_bytes, 64 * 1024**2)
        self.assertEqual(acc.browser.creation.calls, 1)
        seeding.release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(request, 1)
        device = await asyncio.wait_for(waiting, 1)
        self.assertIsNot(device.ctx, context)
        self.assertTrue(context.closed)
        context.add_cookies.assert_not_awaited()
        context.new_page.assert_not_awaited()
        context.page.evaluate.assert_not_awaited()
        self.assert_idle(acc)

    async def test_repeated_cancellation_retains_cleanup_and_other_account_progress(self):
        acc, other = self.account(), self.account("b")
        context = acc.browser.next_context
        page_creation = self.block_method(context.new_page, context.page)
        disposal = self.block(context.disposal)
        request = self.command()
        await asyncio.wait_for(page_creation.entered.wait(), 1)
        request.cancel()
        await asyncio.wait_for(disposal.entered.wait(), 1)
        page_creation.release.set()
        waiting = self.command()
        for _ in range(2):
            request.cancel()
            await self.assert_waiting(request)
        await self.assert_waiting(waiting)
        self.assertEqual(acc.browser.creation.calls, 1)
        self.assertEqual(self.host._reserved_bytes, 64 * 1024**2)
        foreign = await asyncio.wait_for(self.command("b", "device-b"), 1)
        self.assertIs(foreign.ctx, other.browser.created_contexts[0])
        self.assertFalse(await self.host._close_account("a"))
        disposal.release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(request, 1)
        await asyncio.wait_for(waiting, 1)
        self.assertTrue(context.closed)
        context.page.goto.assert_not_awaited()
        context.page.evaluate.assert_not_awaited()
        self.assert_idle(acc)
        self.assert_idle(other)

    async def test_failed_cancelled_preparation_close_cannot_mint_fallback(self):
        acc = self.account()
        context = acc.browser.next_context
        seeding = self.block_method(context.add_init_script)
        context.disposal.failure = RuntimeError("fixture private close payload")
        fallback = acquisition.Page()
        fallback.evaluate = AsyncMock(return_value="fixture-fallback")
        self.host._new_mint_page = AsyncMock(return_value=fallback)
        request = self.command(mint=True)
        await asyncio.wait_for(seeding.entered.wait(), 1)
        request.cancel("device prep cancelled")
        await asyncio.wait_for(context.disposal.entered.wait(), 1)
        seeding.release.set()
        with self.assertRaises(asyncio.CancelledError) as cancelled:
            await asyncio.wait_for(request, 1)
        self.assertEqual(cancelled.exception.args, ("device prep cancelled",))
        self.host._new_mint_page.assert_not_awaited()
        fallback.evaluate.assert_not_awaited()
        context.add_cookies.assert_not_awaited()
        context.page.evaluate.assert_not_awaited()
        self.assertFalse(context.closed)
        self.assertTrue(acc.closing)
        self.assertEqual(acc.lifecycle.phase, "unconfirmed")
        self.events.assert_called_once_with("browser.cleanup", level="warn",
            outcome="unconfirmed", error="cancelled-device-context-close-failed")
        with self.assertRaisesRegex(RuntimeError,
                "^account browser cleanup is unconfirmed; retry after cleanup$"):
            await self.host._account("a")
        self.assertFalse(acc.mint_lock.locked())
        self.assertFalse(acc.lifecycle.rates.state["pending_mint"])
        self.assert_idle(acc)

    async def test_cancelled_warm_keeps_the_device_owned_until_raw_result(self):
        acc = self.account()
        context = acc.browser.next_context
        warming = self.block_method(context.page.evaluate, "fixture-token")
        request = self.command()
        await asyncio.wait_for(warming.entered.wait(), 1)
        request.cancel()
        await self.assert_waiting(request)
        waiting = self.command()
        await self.assert_waiting(waiting)
        self.assertEqual(context.page.evaluate.await_count, 1)
        self.assertEqual(self.host._reserved_bytes, 64 * 1024**2)
        warming.release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(request, 1)
        device = await asyncio.wait_for(waiting, 1)
        self.assertIs(device.ctx, context)
        self.assertTrue(device.warm)
        self.assertEqual(context.page.evaluate.await_count, 2)
        acc.browser.new_context.assert_awaited_once_with()
        self.assertEqual(context.disposal.calls, 0)
        self.assert_idle(acc)

    async def test_closed_page_cleanup_stays_owned_through_cancellation(self):
        acc = self.account()
        device = await asyncio.wait_for(self.command(), 1)
        context = device.ctx
        context.page.closed = True
        disposal = self.block(context.disposal)
        request = self.command()
        await asyncio.wait_for(disposal.entered.wait(), 1)
        request.cancel()
        await self.assert_waiting(request)
        waiting = self.command()
        await self.assert_waiting(waiting)
        self.assertEqual(context.disposal.calls, 1)
        self.assertEqual(acc.browser.creation.calls, 1)
        disposal.release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(request, 1)
        self.assertIs(await asyncio.wait_for(waiting, 1), device)
        self.assertIsNot(device.ctx, context)
        self.assertEqual(context.disposal.calls, 2)
        self.assertTrue(context.closed)
        self.assert_idle(acc)

    async def test_device_owner_does_not_join_unrelated_account_rpc(self):
        acc = self.account()
        unrelated = self.block(acquisition.Call())
        pending = self.start(self.host._rpc("a", unrelated))
        await asyncio.wait_for(unrelated.entered.wait(), 1)
        device = await asyncio.wait_for(self.command(), 1)
        self.assertTrue(device.warm)
        self.assertFalse(pending.done())
        unrelated.release.set()
        await asyncio.wait_for(pending, 1)
        self.assert_idle(acc)

    async def test_failed_preparation_and_close_retain_context_and_forbid_fallback(self):
        acc = self.account()
        context = acc.browser.next_context
        context.add_cookies.side_effect = RuntimeError("fixture prepare failure")
        context.disposal.failure = RuntimeError("fixture private close failure")
        self.host._new_mint_page = AsyncMock()
        with self.assertRaisesRegex(RuntimeError, "browser cleanup unconfirmed"):
            await self.command(mint=True)
        self.assertIs(acc.dev_objs["device-a"].ctx, context)
        self.assertIn(context, acc.browser.contexts)
        self.assertTrue(acc.closing)
        self.assertEqual(acc.lifecycle.phase, "unconfirmed")
        self.host._new_mint_page.assert_not_awaited()
        context.new_page.assert_not_awaited()
        self.events.assert_called_once_with("browser.cleanup", level="warn",
            outcome="unconfirmed", error="device-preparation-close-failed")
        with self.assertRaisesRegex(RuntimeError, "cleanup is unconfirmed"):
            await self.command()
        acc.browser.new_context.assert_awaited_once()
        self.assert_idle(acc)

    async def test_failed_post_preparation_never_uses_ambient_page(self):
        acc = self.account()
        context = acc.browser.next_context
        context.page.goto.side_effect = RuntimeError("fixture navigation failure")
        context.disposal.failure = RuntimeError("fixture close failure")
        self.host._pick_device_sticky = lambda _acc: "device-a"
        self.host._token_acquire = AsyncMock()
        self.host._ensure_editor = AsyncMock()
        with self.assertRaisesRegex(RuntimeError, "browser cleanup unconfirmed"):
            await self.host.handle({"cmd": "ai_post", "account": "a", "body": {}})
        self.assertIs(acc.dev_objs["device-a"].ctx, context)
        self.assertIs(acc.dev_objs["device-a"].page, context.page)
        self.host._token_acquire.assert_not_awaited()
        self.host._ensure_editor.assert_not_awaited()
        self.assertTrue(acc.closing)
        self.assert_idle(acc)

    async def test_returned_cleanup_without_context_closure_is_not_success(self):
        acc = self.account()
        context = acc.browser.next_context
        context.add_init_script.side_effect = RuntimeError("fixture preparation failure")
        context.close = AsyncMock()
        with self.assertRaisesRegex(RuntimeError, "browser cleanup unconfirmed"):
            await self.command()
        self.assertIs(acc.dev_objs["device-a"].ctx, context)
        self.assertTrue(acc.closing)
        self.assertEqual(acc.lifecycle.phase, "unconfirmed")
        self.assert_idle(acc)

    async def test_waiting_device_request_cannot_replace_unconfirmed_context(self):
        acc = self.account()
        context = acc.browser.next_context
        gate = self.block(acquisition.Call())
        async def fail_after_wait(*_args):
            await gate()
            raise RuntimeError("fixture preparation failure")
        context.add_init_script.side_effect = fail_after_wait
        context.disposal.failure = RuntimeError("fixture close failure")
        first = self.command()
        await asyncio.wait_for(gate.entered.wait(), 1)
        waiting = self.command()
        await self.assert_waiting(waiting)
        gate.release.set()
        for request in (first, waiting):
            with self.assertRaisesRegex(RuntimeError, "browser cleanup unconfirmed"):
                await asyncio.wait_for(request, 1)
        acc.browser.new_context.assert_awaited_once()
        self.assertIs(acc.dev_objs["device-a"].ctx, context)
        self.assertTrue(acc.closing)
        self.assert_idle(acc)
