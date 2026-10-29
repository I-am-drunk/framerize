"""Cancelled device context creation with inert browser ports."""
import asyncio
from dataclasses import replace
import unittest
from unittest.mock import AsyncMock, Mock, call, patch

import test_page_acquisition as acquisition


class DeviceContext:
    def __init__(self):
        self.disposal, self.closed = acquisition.Call(), False
        self.page = acquisition.Page()
        self.pages = [self.page]
        self.page.goto = AsyncMock(wraps=self.page.goto)
        self.page.evaluate = AsyncMock(return_value="fixture-token")
        self.add_init_script = AsyncMock()
        self.add_cookies = AsyncMock()
        self.new_page = AsyncMock(return_value=self.page)
        self.calls = Mock()
        for name, method in (("script", self.add_init_script),
                ("cookies", self.add_cookies), ("page", self.new_page),
                ("goto", self.page.goto), ("evaluate", self.page.evaluate)):
            self.calls.attach_mock(method, name)

    async def close(self):
        await self.disposal()
        self.closed = self.page.closed = True
        if self in self.browser.contexts:
            self.browser.contexts.remove(self)


class DeviceContextCreationTests(unittest.IsolatedAsyncioTestCase):
    finish = acquisition.PageAcquisitionTests.finish
    block = acquisition.PageAcquisitionTests.block
    start = acquisition.PageAcquisitionTests.start
    assert_waiting = acquisition.PageAcquisitionTests.assert_waiting
    assert_idle = acquisition.PageAcquisitionTests.assert_idle

    def setUp(self):
        acquisition.PageAcquisitionTests.setUp(self)
        from framerize_host import events
        guard = patch.object(events, "emit", Mock())
        guard.start()
        self.addCleanup(guard.stop)
        self.events = events.emit

    def account(self, name="a"):
        acc = acquisition.PageAcquisitionTests.account(self, name)
        acc.session_cookie = ("session", "fixture-" + name)
        acc.ctx.add_cookies = AsyncMock()
        acc.lifecycle.rates.policy = replace(acc.lifecycle.rates.policy, mint_gap=0)
        browser = acc.browser
        browser.creation = acquisition.Call()
        browser.next_context, browser.created_contexts = DeviceContext(), []
        async def create():
            await browser.creation()
            context = browser.next_context
            context.browser = browser
            browser.next_context = DeviceContext()
            browser.created_contexts.append(context)
            browser.contexts.append(context)
            return context
        browser.new_context = AsyncMock(side_effect=create)
        return acc

    async def prepare(self, name="a", device="device-a", mint=False):
        async with self.host._use_account(name) as acc:
            if mint:
                return await self.host._mint(acc, acc.page, name, device)
            return await self.host._dev_page(acc, name, device)

    def command(self, name="a", device="device-a", mint=False):
        task = self.start(self.prepare(name, device, mint))
        key = len(self.waiters)
        self.host._command_tasks[key] = task
        task.add_done_callback(lambda _: self.host._command_tasks.pop(key, None))
        return task

    def assert_unprepared(self, context):
        self.assertEqual(context.calls.mock_calls, [])
        self.assertEqual(context.page.waits, [])

    async def test_cancelled_creation_closes_late_context_before_unlocking(self):
        acc = self.account()
        context = acc.browser.next_context
        creation = self.block(acc.browser.creation)
        request = self.command()
        await asyncio.wait_for(creation.entered.wait(), 1)
        request.cancel("cancelled context")
        await self.assert_waiting(request)
        device = acc.dev_objs["device-a"]
        self.assertTrue(device.lock.locked())
        self.assertEqual(self.host._reserved_bytes, 64 * 1024**2)
        creation.release.set()
        with self.assertRaises(asyncio.CancelledError) as cancelled:
            await asyncio.wait_for(request, 1)
        self.assertEqual(cancelled.exception.args, ("cancelled context",))
        self.assertTrue(context.closed)
        self.assertEqual(context.disposal.calls, 1)
        self.assert_unprepared(context)
        self.assertIsNone(device.ctx)
        self.assertIsNone(device.page)
        self.assertFalse(device.warm)
        self.assertFalse(device.lock.locked())
        self.assertEqual(acc.lifecycle.phase, "resident")
        self.assertFalse(acc.closing)
        self.events.assert_not_called()
        self.assert_idle(acc)

    async def test_repeated_cancellation_retains_device_lock_and_close(self):
        acc, other = self.account(), self.account("b")
        context = acc.browser.next_context
        creation = self.block(acc.browser.creation)
        disposal = self.block(context.disposal)
        request = self.command()
        await asyncio.wait_for(creation.entered.wait(), 1)
        request.cancel()
        creation.release.set()
        await asyncio.wait_for(disposal.entered.wait(), 1)
        waiting = self.command()
        for _ in range(2):
            request.cancel()
            await self.assert_waiting(request)
        await self.assert_waiting(waiting)
        self.assertEqual(acc.browser.creation.calls, 1)
        self.assertTrue(acc.dev_objs["device-a"].lock.locked())
        self.assertEqual(self.host._reserved_bytes, 64 * 1024**2)
        foreign = await asyncio.wait_for(self.command("b", "device-b"), 1)
        self.assertIs(foreign.ctx, other.browser.created_contexts[0])
        self.assertFalse(await self.host._close_account("a"))
        disposal.release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(request, 1)
        device = await asyncio.wait_for(waiting, 1)
        self.assertIs(device.ctx, acc.browser.created_contexts[1])
        self.assertTrue(context.closed)
        self.assert_unprepared(context)
        self.assert_idle(acc)
        self.assert_idle(other)

    async def test_late_creation_error_preserves_cancellation_without_close(self):
        acc = self.account()
        context = acc.browser.next_context
        creation = self.block(acc.browser.creation)
        creation.failure = RuntimeError("fixture creation failed")
        request = self.command()
        await asyncio.wait_for(creation.entered.wait(), 1)
        request.cancel()
        creation.release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(request, 1)
        await asyncio.wait_for(asyncio.gather(*acc.lifecycle.activity.pending,
            return_exceptions=True), 1)
        self.assertEqual(acc.browser.created_contexts, [])
        self.assertEqual(context.disposal.calls, 0)
        self.assert_unprepared(context)
        self.assertIsNone(acc.dev_objs["device-a"].ctx)
        self.assertEqual(acc.lifecycle.phase, "resident")
        self.events.assert_not_called()
        self.assert_idle(acc)

    async def test_failed_late_close_preserves_cancellation_and_account_fence(self):
        acc = self.account()
        context = acc.browser.next_context
        creation = self.block(acc.browser.creation)
        context.disposal.failure = RuntimeError("fixture private failure payload")
        self.host._new_mint_page = AsyncMock(side_effect=AssertionError("fallback"))
        request = self.command(mint=True)
        await asyncio.wait_for(creation.entered.wait(), 1)
        request.cancel("cancelled device mint")
        await self.assert_waiting(request)
        creation.release.set()
        with self.assertRaises(asyncio.CancelledError) as cancelled:
            await asyncio.wait_for(request, 1)
        self.assertEqual(cancelled.exception.args, ("cancelled device mint",))
        self.assertFalse(context.closed)
        self.assertEqual(context.disposal.calls, 1)
        self.assert_unprepared(context)
        self.host._new_mint_page.assert_not_awaited()
        self.assertEqual(self.host._browser_status("a")["state"], "unconfirmed")
        self.assertTrue(acc.closing)
        self.events.assert_called_once_with("browser.cleanup", level="warn",
            outcome="unconfirmed", error="cancelled-device-context-close-failed")
        with self.assertRaisesRegex(RuntimeError,
                "^account browser cleanup is unconfirmed; retry after cleanup$"):
            await self.host._account("a")
        self.assertFalse(acc.mint_lock.locked())
        self.assertFalse(acc.lifecycle.rates.state["pending_mint"])
        self.assert_idle(acc)
        other = self.account("b")
        device = await asyncio.wait_for(self.command("b", "device-b"), 1)
        self.assertIs(device.ctx, other.browser.created_contexts[0])
        self.assertTrue(await asyncio.wait_for(self.host._close_account("a"), 1))
        self.assertTrue(acc.browser.acknowledged.is_set())
        self.assertEqual(acc.lifecycle.phase, "absent")
        self.assertTrue(other.browser.is_connected())

    async def test_normal_preparation_preserves_cookie_order_and_device_reuse(self):
        acc = self.account()
        context = acc.browser.next_context
        device = await asyncio.wait_for(self.command(), 1)
        self.assertIs(await asyncio.wait_for(self.command(), 1), device)
        self.assertIs(device.ctx, context)
        self.assertIs(device.page, context.page)
        self.assertTrue(device.warm)
        acc.browser.new_context.assert_awaited_once_with()
        self.assertEqual([entry[0] for entry in context.calls.mock_calls],
                         ["script", "cookies", "page", "goto", "evaluate"])
        context.add_init_script.assert_awaited_once_with(
            'try { localStorage.setItem(\'__cuid\', "device-a"); } catch(e) {}')
        context.add_cookies.assert_awaited_once_with([
            {"name": "__cuid", "value": "device-a", "domain": ".framer.com",
             "path": "/", "secure": True, "httpOnly": False, "sameSite": "None"},
            {"name": "session", "value": "fixture-a", "domain": ".framer.com",
             "path": "/", "secure": True, "httpOnly": True, "sameSite": "None"}])
        context.page.goto.assert_awaited_once_with(self.module.LOGIN_URL,
            wait_until="domcontentloaded", timeout=90000)
        context.page.evaluate.assert_awaited_once_with(self.module.WARM_JS)
        self.assertEqual(context.disposal.calls, 0)
        self.assert_idle(acc)

    async def test_normal_creation_and_preparation_errors_are_preserved(self):
        for stage in ("creation", "add_init_script", "add_cookies", "new_page", "goto"):
            with self.subTest(stage=stage):
                acc = self.account(stage)
                context = acc.browser.next_context
                error = RuntimeError("fixture " + stage)
                if stage == "creation":
                    acc.browser.creation.failure = error
                elif stage == "goto":
                    context.page.goto.side_effect = error
                else:
                    getattr(context, stage).side_effect = error
                with self.assertRaises(RuntimeError) as raised:
                    await asyncio.wait_for(self.command(stage), 1)
                self.assertIs(raised.exception, error)
                self.assertEqual(context.disposal.calls, 0 if stage == "creation" else 1)
                self.assertIsNone(acc.dev_objs["device-a"].ctx)
                self.assertIsNone(acc.dev_objs["device-a"].page)
                self.assertFalse(acc.closing)
                self.assertEqual(acc.lifecycle.phase, "resident")
                context.page.evaluate.assert_not_awaited()
                self.assert_idle(acc)

    async def test_warm_failure_retries_on_the_same_published_context(self):
        acc = self.account()
        context = acc.browser.next_context
        context.page.evaluate.side_effect = RuntimeError("fixture warm failure")
        device = await asyncio.wait_for(self.command(), 1)
        self.assertIs(device.ctx, context)
        self.assertFalse(device.warm)
        context.page.evaluate.side_effect = None
        self.assertIs(await asyncio.wait_for(self.command(), 1), device)
        self.assertTrue(device.warm)
        acc.browser.new_context.assert_awaited_once_with()
        self.assertEqual(context.page.evaluate.await_args_list,
                         [call(self.module.WARM_JS), call(self.module.WARM_JS)])
        self.assertEqual(context.disposal.calls, 0)
        self.assert_idle(acc)

    async def test_session_seed_waits_for_cancelled_context_close_then_seeds_retry(self):
        acc = self.account()
        context = acc.browser.next_context
        creation = self.block(acc.browser.creation)
        disposal = self.block(context.disposal)
        request = self.command()
        await asyncio.wait_for(creation.entered.wait(), 1)
        request.cancel()
        creation.release.set()
        await asyncio.wait_for(disposal.entered.wait(), 1)
        seeding = self.start(self.host.handle({"cmd": "set_cookie", "account": "a",
            "name": "session", "value": "fixture-new"}))
        await self.assert_waiting(seeding)
        self.assertEqual(acc.session_cookie, ("session", "fixture-new"))
        self.assert_unprepared(context)
        disposal.release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(request, 1)
        self.assertTrue(await asyncio.wait_for(seeding, 1))
        device = await asyncio.wait_for(self.command(), 1)
        self.assertIsNot(device.ctx, context)
        cookies = device.ctx.add_cookies.await_args.args[0]
        self.assertEqual({item["name"]: item["value"] for item in cookies},
                         {"__cuid": "device-a", "session": "fixture-new"})
        self.assertTrue(context.closed)
        self.assert_idle(acc)

    async def test_shutdown_joins_cancelled_context_close_before_browser_close(self):
        acc = self.account()
        context = acc.browser.next_context
        creation = self.block(acc.browser.creation)
        disposal = self.block(context.disposal)
        request = self.command()
        await asyncio.wait_for(creation.entered.wait(), 1)
        request.cancel()
        creation.release.set()
        await asyncio.wait_for(disposal.entered.wait(), 1)
        closing = self.start(self.host.close())
        await self.assert_waiting(closing)
        request.cancel()
        await self.assert_waiting(request)
        self.assertEqual(acc.browser.calls, 0)
        disposal.release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(request, 1)
        await asyncio.wait_for(closing, 1)
        self.assertTrue(context.closed)
        self.assertTrue(acc.browser.acknowledged.is_set())
        self.assertEqual(acc.lifecycle.phase, "absent")
        self.assert_unprepared(context)
        self.assert_idle(acc)
