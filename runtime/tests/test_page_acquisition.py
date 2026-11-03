"""Pool acquisition ownership through inert, controllable browser calls."""
import asyncio
import unittest
from unittest.mock import AsyncMock, patch

import test_browser as browser_fixture
from test_lifecycle import FakeBrowser


class Call:
    def __init__(self):
        self.entered, self.release = asyncio.Event(), asyncio.Event()
        self.release.set()
        self.failure, self.calls = None, 0

    async def __call__(self):
        self.calls += 1
        self.entered.set()
        await self.release.wait()
        if self.failure is not None:
            raise self.failure


class Page:
    def __init__(self):
        self.navigation, self.preparation, self.disposal = Call(), Call(), Call()
        self.gotos, self.waits, self.listeners = [], [], []
        self.closed = False

    def is_closed(self):
        return self.closed

    async def goto(self, url, **options):
        self.gotos.append((url, options))
        await self.navigation()

    async def wait_for_timeout(self, milliseconds):
        self.waits.append(milliseconds)
        await self.preparation()

    async def close(self):
        await self.disposal()
        self.closed = True
        for listener in tuple(self.listeners):
            listener()

    def on(self, event, listener):
        assert event == "close"
        self.listeners.append(listener)

    def remove_listener(self, event, listener):
        assert event == "close"
        self.listeners.remove(listener)


class Context:
    def __init__(self):
        self.creation = Call()
        self.ambient, self.next_page = Page(), Page()
        self.pages = [self.ambient]

    async def new_page(self):
        await self.creation()
        self.pages.append(self.next_page)
        return self.next_page


class PageAcquisitionTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        browser_fixture.LazyBrowserRuntime.setUp(self)
        self.waiters, self.gates = [], []
        self.host._launch = AsyncMock(side_effect=AssertionError("unexpected launch"))
        for target, name, value in (
                (self.module, "emit", lambda *args, **kwargs: None),
                (self.module, "trace", lambda *args, **kwargs: None),
                (self.driver, "verify_close_protocol", lambda: None),
                (self.module, "memory_snapshot", lambda: {}),
                (self.module, "POOL_CAP", 2)):
            guard = patch.object(target, name, value)
            guard.start()
            self.addCleanup(guard.stop)
        self.addAsyncCleanup(self.finish)

    async def finish(self):
        for gate in self.gates:
            gate.release.set()
        for task in self.waiters:
            if not task.done():
                task.cancel()
        await asyncio.wait_for(asyncio.gather(*self.waiters, return_exceptions=True), 2)
        for life in self.host.lifecycles.values():
            await asyncio.wait_for(asyncio.gather(*life.activity.pending,
                return_exceptions=True), 2)
        await asyncio.wait_for(self.host.close(), 2)

    def account(self, name="a"):
        life, ctx, browser = self.module.AccountLifecycle(), Context(), FakeBrowser()
        life.browser, life.phase = browser, "resident"
        life.close_receipt = self.module.BrowserCloseReceipt(browser)
        browser.contexts = [ctx]
        acc = self.module.Account(None, ctx, ctx.ambient, life)
        acc.browser = browser
        self.host.accounts[name], self.host.lifecycles[name] = acc, life
        return acc

    def block(self, gate):
        gate.release.clear()
        self.gates.append(gate)
        return gate

    async def acquire(self, name="a"):
        async with self.host._use_account(name):
            return await self.host._acquire(name)

    def start(self, coroutine):
        task = asyncio.create_task(coroutine)
        self.waiters.append(task)
        return task

    def start_acquire(self, name="a"):
        task = self.start(self.acquire(name))
        key = len(self.waiters)
        self.host._command_tasks[key] = task
        task.add_done_callback(lambda _: self.host._command_tasks.pop(key, None))
        return task

    async def assert_waiting(self, task):
        await asyncio.wait({task}, timeout=0.02)
        self.assertFalse(task.done(), "request finished before its page was settled")

    def assert_idle(self, acc):
        self.assertFalse(acc.lock.locked())
        self.assertFalse(acc.lifecycle.activity.pending)
        self.assertFalse(acc.lifecycle.activity.activities)
        self.assertEqual(acc.lifecycle.active_commands, 0)
        self.assertEqual(acc.active_requests, 0)
        self.assertEqual(self.host._reserved_bytes, 0)

    async def test_cancelled_creation_recycles_late_page_before_unlocking(self):
        acc = self.account()
        ambient, _, _ = await self.acquire()
        creation = self.block(acc.ctx.creation)
        request = self.start_acquire()
        await asyncio.wait_for(creation.entered.wait(), 1)
        request.cancel("abandoned page")
        await self.assert_waiting(request)
        self.assertTrue(acc.lock.locked())
        self.assertEqual(self.host._reserved_bytes, 64 * 1024**2)
        creation.release.set()
        with self.assertRaises(asyncio.CancelledError) as cancelled:
            await asyncio.wait_for(request, 1)
        self.assertEqual(str(cancelled.exception), "abandoned page")
        page = acc.ctx.next_page
        self.assertEqual(acc.size, 2)
        self.assertEqual(acc.borrowed, {ambient})
        self.assertEqual(acc.pool.qsize(), 1)
        self.assertEqual(page.waits, [800])
        self.assertFalse(page.is_closed())
        self.assertIs((await self.acquire())[0], page)
        self.assertEqual(creation.calls, 1)
        self.host._release(acc, page)
        self.assert_idle(acc)

    async def test_repeated_cancellation_keeps_lock_and_other_account_progress(self):
        acc, other = self.account(), self.account("b")
        await self.acquire()
        page = acc.ctx.next_page
        navigation, preparation = self.block(page.navigation), self.block(page.preparation)
        request = self.start_acquire()
        await asyncio.wait_for(navigation.entered.wait(), 1)
        same_account = self.start_acquire()
        for _ in range(2):
            request.cancel()
            await self.assert_waiting(request)
        await self.assert_waiting(same_account)
        self.assertTrue(acc.lock.locked())
        self.assertEqual(acc.ctx.creation.calls, 1)
        self.assertFalse(await self.host._close_account("a"))
        await asyncio.wait_for(self.acquire("b"), 1)
        other_page, _, _ = await asyncio.wait_for(self.acquire("b"), 1)
        self.assertIs(other_page, other.ctx.next_page)
        self.assertEqual(other.size, 2)
        self.assertNotIn(page, other.borrowed)
        navigation.release.set()
        await asyncio.wait_for(preparation.entered.wait(), 1)
        request.cancel()
        await self.assert_waiting(request)
        self.assertTrue(acc.lock.locked())
        self.assertEqual(self.host._reserved_bytes, 64 * 1024**2)
        preparation.release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(request, 1)
        self.assertIs((await asyncio.wait_for(same_account, 1))[0], page)
        self.assertEqual(acc.ctx.creation.calls, 1)
        self.assertEqual(acc.size, 2)
        self.assertEqual(page.disposal.calls, 0)
        self.assert_idle(acc)

    async def test_cancelled_navigation_failure_joins_close_before_unlocking(self):
        acc = self.account()
        ambient, _, _ = await self.acquire()
        page = acc.ctx.next_page
        navigation, disposal = self.block(page.navigation), self.block(page.disposal)
        navigation.failure = RuntimeError("fixture navigation failed")
        request = self.start_acquire()
        await asyncio.wait_for(navigation.entered.wait(), 1)
        request.cancel()
        await self.assert_waiting(request)
        navigation.release.set()
        await asyncio.wait_for(disposal.entered.wait(), 1)
        request.cancel()
        await self.assert_waiting(request)
        self.assertTrue(acc.lock.locked())
        self.assertEqual(self.host._reserved_bytes, 64 * 1024**2)
        disposal.release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(request, 1)
        self.assertTrue(page.is_closed())
        self.assertEqual(page.disposal.calls, 1)
        self.assertEqual(acc.size, 1)
        self.assertEqual(acc.borrowed, {ambient})
        self.assertTrue(acc.pool.empty())
        self.assert_idle(acc)

    async def test_normal_acquisition_preserves_preparation_cap_and_queue(self):
        acc = self.account()
        ambient, _, initial_wait = await self.acquire()
        page, owner, new_wait = await self.acquire()
        self.assertIs(owner, acc)
        self.assertEqual((initial_wait, new_wait), (0.0, 0.0))
        self.assertEqual(page.gotos, [(self.module.LOGIN_URL,
            {"wait_until": "domcontentloaded", "timeout": 60000})])
        self.assertEqual(page.waits, [800])
        self.assertEqual(acc.size, 2)
        self.assertEqual(acc.borrowed, {ambient, page})
        self.assertEqual(len(acc.tasks), 0)
        waiting = self.start_acquire()
        await self.assert_waiting(waiting)
        self.assertFalse(acc.lock.locked())
        self.assertEqual(acc.ctx.creation.calls, 1)
        self.host._release(acc, page)
        reused, _, queue_wait = await asyncio.wait_for(waiting, 1)
        self.assertIs(reused, page)
        self.assertGreater(queue_wait, 0.0)
        self.assertEqual(acc.ctx.creation.calls, 1)
        self.assert_idle(acc)

    async def test_failures_preserve_original_error_and_pool_capacity(self):
        for stage in ("creation", "navigation", "preparation"):
            with self.subTest(stage=stage):
                acc = self.account(stage)
                ambient, _, _ = await self.acquire(stage)
                page = acc.ctx.next_page
                failing = acc.ctx.creation if stage == "creation" else getattr(page, stage)
                error = RuntimeError("fixture " + stage)
                failing.failure = error
                with self.assertRaises(RuntimeError) as raised:
                    await asyncio.wait_for(self.acquire(stage), 1)
                if stage == 'navigation':
                    self.assertIs(raised.exception.__cause__, error)
                    self.assertTrue(str(raised.exception).startswith('browser preparation failed:'))
                    self.assertIn(str(error), str(raised.exception))
                else:
                    self.assertIs(raised.exception, error)
                self.assertEqual(page.disposal.calls, 0 if stage == "creation" else 1)
                self.assertEqual(acc.size, 1)
                self.assertEqual(acc.borrowed, {ambient})
                self.assertTrue(acc.pool.empty())
                self.assert_idle(acc)

    async def test_cancelled_lock_wait_does_not_allocate_or_consume_page(self):
        acc = self.account()
        async with acc.lock:
            waiting = self.start_acquire()
            await self.assert_waiting(waiting)
            waiting.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await asyncio.wait_for(waiting, 1)
            self.assertEqual(acc.ctx.creation.calls, 0)
            self.assertEqual(acc.pool.qsize(), 1)
            self.assertFalse(acc.borrowed)
        self.assertIs((await self.acquire())[0], acc.ctx.ambient)
        self.assert_idle(acc)

    async def test_cancelled_queue_wait_leaves_pool_pages_available(self):
        acc = self.account()
        ambient, _, _ = await self.acquire()
        page, _, _ = await self.acquire()
        waiting = self.start_acquire()
        await self.assert_waiting(waiting)
        waiting.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(waiting, 1)
        self.assertEqual(acc.borrowed, {ambient, page})
        self.assertTrue(acc.pool.empty())
        self.host._release(acc, page)
        self.assertIs((await self.acquire())[0], page)
        self.assertEqual(acc.ctx.creation.calls, 1)
        self.assertEqual(acc.size, 2)
        self.assert_idle(acc)

    async def test_memory_refusal_preserves_pool_and_reservation_accounting(self):
        acc = self.account()
        ambient, _, _ = await self.acquire()
        with patch.object(self.module, "memory_snapshot",
                          return_value={"allocation_headroom_bytes": 1}):
            with self.assertRaises(self.module.MemoryPressureError):
                await self.acquire()
        self.assertEqual(acc.ctx.creation.calls, 0)
        self.assertEqual(acc.size, 1)
        self.assertEqual(acc.borrowed, {ambient})
        self.assertTrue(acc.pool.empty())
        self.assert_idle(acc)

    async def test_shutdown_joins_cancelled_preparation_before_browser_close(self):
        acc = self.account()
        await self.acquire()
        creation = self.block(acc.ctx.creation)
        preparation = self.block(acc.ctx.next_page.preparation)
        request = self.start_acquire()
        await asyncio.wait_for(creation.entered.wait(), 1)
        request.cancel()
        closing = self.start(self.host.close())
        await self.assert_waiting(closing)
        self.assertEqual(acc.browser.calls, 0)
        creation.release.set()
        await asyncio.wait_for(preparation.entered.wait(), 1)
        await self.assert_waiting(closing)
        self.assertEqual(acc.browser.calls, 0)
        self.assertTrue(acc.lock.locked())
        preparation.release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(request, 1)
        await asyncio.wait_for(closing, 1)
        self.assertEqual(acc.browser.calls, 1)
        self.assertTrue(acc.browser.acknowledged.is_set())
        self.assertEqual(acc.lifecycle.phase, "absent")
        self.assertFalse(self.host.accounts)
        self.assertFalse(acc.tasks)
        self.assertFalse(acc.borrowed)
        self.assert_idle(acc)
