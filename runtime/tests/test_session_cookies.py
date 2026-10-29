"""Session propagation through real host ownership with inert browser ports."""
import asyncio
from contextlib import asynccontextmanager
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, patch

import test_browser as browser_fixture
from test_lifecycle import FakeBrowser


class CookieContext:
    def __init__(self, session=None):
        self.cookies = {} if session is None else {"session": session}
        self.calls, self.gates, self.failure = [], {}, None
        self.page = SimpleNamespace(is_closed=lambda: False,
            goto=AsyncMock(), evaluate=AsyncMock(return_value=True))
        self.add_init_script = AsyncMock()
        self.new_page = AsyncMock(return_value=self.page)
        self.close = AsyncMock()

    async def add_cookies(self, cookies):
        self.calls.append([dict(cookie) for cookie in cookies])
        for cookie in cookies:
            gate = self.gates.get((cookie["name"], cookie["value"]))
            if gate is not None:
                entered, release = gate
                entered.set()
                await release.wait()
        if self.failure is not None:
            raise self.failure
        self.cookies.update((cookie["name"], cookie["value"]) for cookie in cookies)


class SessionCookieTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        browser_fixture.LazyBrowserRuntime.setUp(self)
        self.waiters, self.releases = [], []
        self.host._launch = AsyncMock(side_effect=AssertionError("unexpected launch"))
        for target, name in ((self.module, "emit"), (self.module, "trace"),
                             (self.driver, "verify_close_protocol")):
            guard = patch.object(target, name, lambda *args, **kwargs: None)
            guard.start()
            self.addCleanup(guard.stop)

        @asynccontextmanager
        async def allocation(_estimate):
            yield

        self.host._allocation = allocation
        self.addAsyncCleanup(self.finish)

    async def finish(self):
        for release in self.releases:
            release.set()
        await asyncio.wait_for(asyncio.gather(*self.waiters, return_exceptions=True), 2)
        for life in self.host.lifecycles.values():
            await asyncio.wait_for(asyncio.gather(*life.activity.pending,
                return_exceptions=True), 2)
        await asyncio.wait_for(self.host.close(), 2)

    def account(self, account="a", session="old-a"):
        life = self.module.AccountLifecycle()
        browser, ctx = FakeBrowser(), CookieContext(session)
        browser.new_context = AsyncMock(side_effect=AssertionError("unexpected context"))
        life.browser, life.phase = browser, "resident"
        life.close_receipt = self.module.BrowserCloseReceipt(browser)
        acc = self.module.Account(None, ctx, ctx.page, life)
        acc.browser, acc.session_cookie = browser, ("session", session)
        self.host.accounts[account], self.host.lifecycles[account] = acc, life
        return acc

    def device(self, acc, name="device-a"):
        device = self.module.Device(name)
        device.ctx = CookieContext(acc.session_cookie[1])
        device.page, device.warm = device.ctx.page, True
        acc.dev_objs[name] = device
        return device

    def start(self, coroutine):
        task = asyncio.create_task(coroutine)
        self.waiters.append(task)
        return task

    def gate(self, ctx, value):
        entered, release = asyncio.Event(), asyncio.Event()
        self.releases.append(release)
        ctx.gates[("session", value)] = (entered, release)
        return entered, release

    async def seed(self, value, account="a", name="session"):
        return await asyncio.wait_for(self.host.handle(dict(
            cmd="set_cookie", account=account, name=name, value=value)), 2)

    def assert_session(self, acc, value):
        self.assertEqual(acc.session_cookie, ("session", value))
        self.assertEqual(acc.ctx.cookies["session"], value)
        for device in acc.dev_objs.values():
            self.assertEqual(device.ctx.cookies["session"], value)

    async def test_device_failure_rejects_seed_and_retry_repairs_all_contexts(self):
        acc = self.account()
        first, failed = self.device(acc, "first"), self.device(acc, "failed")
        failed.ctx.failure = RuntimeError("device cookie write refused")
        with self.assertRaisesRegex(RuntimeError, "^device cookie write refused$"):
            await self.seed("new-a")
        self.assertEqual(acc.ctx.cookies["session"], "new-a")
        self.assertEqual(first.ctx.cookies["session"], "new-a")
        self.assertEqual(failed.ctx.cookies["session"], "old-a")
        failed.ctx.failure = None
        self.assertTrue(await self.seed("new-a"))
        self.assert_session(acc, "new-a")
        self.assertEqual([len(ctx.calls) for ctx in (acc.ctx, first.ctx, failed.ctx)],
                         [2, 2, 2])
        self.assertFalse(acc.lifecycle.activity.pending)
        self.assertFalse(acc.lifecycle.activity.activities)

    async def test_seed_waits_for_inflight_device_publication(self):
        acc, ctx = self.account(), CookieContext()
        entered, release = asyncio.Event(), asyncio.Event()
        self.releases.append(release)

        async def new_page():
            entered.set()
            await release.wait()
            return ctx.page

        ctx.new_page = AsyncMock(side_effect=new_page)
        acc.browser.new_context = AsyncMock(return_value=ctx)
        creating = self.start(self.host._dev_page(acc, "a", "starting-device"))
        await asyncio.wait_for(entered.wait(), 1)
        self.assertEqual(ctx.cookies["session"], "old-a")
        device = acc.dev_objs["starting-device"]
        self.assertIs(device.ctx, ctx)
        self.assertIsNone(device.page)
        self.assertTrue(device.lock.locked())
        seeding = self.start(self.seed("new-a"))
        with self.assertRaises(asyncio.TimeoutError):
            await asyncio.wait_for(asyncio.shield(seeding), 0.02)
        self.assertEqual(acc.ctx.cookies["session"], "new-a")
        self.assertEqual(ctx.cookies["session"], "old-a")
        release.set()
        device = await asyncio.wait_for(creating, 1)
        self.assertTrue(await seeding)
        self.assertIs(device.ctx, ctx)
        self.assert_session(acc, "new-a")
        self.assertEqual(ctx.cookies["__cuid"], "starting-device")

    async def test_future_device_inherits_the_acknowledged_session(self):
        acc = self.account()
        self.assertTrue(await self.seed("new-a"))
        ctx = CookieContext()
        acc.browser.new_context = AsyncMock(return_value=ctx)
        device = await asyncio.wait_for(self.host._dev_page(acc, "a", "future"), 1)
        self.assertIs(device.ctx, ctx)
        self.assert_session(acc, "new-a")
        self.assertEqual(ctx.cookies["__cuid"], "future")

    async def test_ambient_failure_preserves_session_and_skips_device_writes(self):
        acc = self.account()
        device = self.device(acc)
        acc.ctx.failure = RuntimeError("ambient cookie write refused")
        with self.assertRaisesRegex(RuntimeError, "^ambient cookie write refused$"):
            await self.seed("new-a")
        self.assert_session(acc, "old-a")
        self.assertEqual(device.ctx.calls, [])

    async def test_non_session_cookie_keeps_existing_scope(self):
        acc = self.account()
        device = self.device(acc)
        self.assertTrue(await self.seed("dark", name="theme"))
        self.assertEqual(acc.ctx.cookies["theme"], "dark")
        self.assert_session(acc, "old-a")
        self.assertNotIn("theme", device.ctx.cookies)
        self.assertEqual(device.ctx.calls, [])

    async def ordered_updates(self, cancel_first):
        acc, other = self.account(), self.account("b", "old-b")
        device, other_device = self.device(acc), self.device(other, "device-b")
        entered, release = self.gate(device.ctx, "first-a")
        first = self.start(self.seed("first-a"))
        await asyncio.wait_for(entered.wait(), 1)
        self.assertEqual(acc.ctx.cookies["session"], "first-a")
        self.assertEqual(device.ctx.cookies["session"], "old-a")
        if cancel_first:
            first.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await first
        self.assertTrue(acc.lifecycle.activity.pending)
        self.assertFalse(await self.host._close_account("a"))
        self.assertTrue(acc.browser.connected)
        second = self.start(self.seed("second-a"))
        self.assertTrue(await self.seed("new-b", account="b"))
        self.assert_session(other, "new-b")
        with self.assertRaises(asyncio.TimeoutError):
            await asyncio.wait_for(asyncio.shield(second), 0.02)
        release.set()
        if not cancel_first:
            self.assertTrue(await first)
        self.assertTrue(await second)
        self.assert_session(acc, "second-a")
        self.assert_session(other, "new-b")
        self.assertEqual(len(other_device.ctx.calls), 1)
        self.assertFalse(acc.lifecycle.activity.pending)
        self.assertFalse(acc.lifecycle.activity.activities)
        self.assertTrue(await self.host._close_account("a"))
        self.assertFalse(acc.browser.connected)
        self.assertTrue(other.browser.connected)
        self.host._launch.assert_not_called()

    async def test_cancelled_seed_retains_owner_and_orders_later_session(self):
        await self.ordered_updates(cancel_first=True)

    async def test_overlapping_seeds_preserve_order_and_account_isolation(self):
        await self.ordered_updates(cancel_first=False)
