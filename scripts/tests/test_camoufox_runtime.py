"""Offline regressions for ownership, cancellation and memory admission."""
import asyncio
import importlib.util
import io
import queue
import sys
import tempfile
import time
import types
import unittest
from pathlib import Path
from unittest.mock import AsyncMock, Mock, patch

ROOT = Path(__file__).resolve().parents[1]
camoufox = types.ModuleType("camoufox.async_api")
camoufox.AsyncNewBrowser = AsyncMock()
playwright = types.ModuleType("playwright.async_api")
playwright.async_playwright = Mock()
with patch.dict(sys.modules, {"camoufox.async_api": camoufox, "playwright.async_api": playwright}):
    spec = importlib.util.spec_from_file_location("runtime_under_test", ROOT / "camoufox_host.py")
    runtime = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(runtime)


class Page:
    def __init__(self):
        self.closed = False
        self.url = "https://framer.com/login"
        self.goto = AsyncMock()
        self.wait_for_timeout = AsyncMock()
        self.wait_for_selector = AsyncMock()
        self.evaluate = AsyncMock(return_value=True)

    def is_closed(self):
        return self.closed

    async def close(self):
        self.closed = True


class Context:
    def __init__(self):
        self.pages = []
        self.closed = False
        self.add_init_script = AsyncMock()
        self.add_cookies = AsyncMock()
        self.cookies = AsyncMock(return_value=[{"name": "session", "value": "fixture-session"}])

    async def new_page(self):
        await asyncio.sleep(0)
        page = Page()
        self.pages.append(page)
        return page

    async def close(self):
        self.closed = True
        for page in self.pages:
            await page.close()


class Browser:
    def __init__(self):
        self.contexts = []
        self.closed = False

    async def new_context(self):
        await asyncio.sleep(0)
        ctx = Context()
        self.contexts.append(ctx)
        return ctx

    async def close(self):
        self.closed = True
        for ctx in self.contexts:
            await ctx.close()


class RuntimeTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        for name, value in (("ROOT", self.tmp.name), ("trace", Mock()),
                            ("memory_snapshot", Mock(return_value={})), ("_cf", None)):
            p = patch.object(runtime, name, value)
            p.start()
            self.addCleanup(p.stop)
        self.host = runtime.Host()
        self.host.idle_timeout = 0
        self.addAsyncCleanup(self.host.close)

    async def account(self, name="account-a"):
        browser = Browser()
        ctx = await browser.new_context()
        page = await ctx.new_page()
        acc = runtime.Account(None, ctx, page)
        acc.browser = browser
        self.host.accounts[name] = acc
        return acc

    async def test_many_commands_return_their_page(self):
        acc = await self.account()
        for _ in range(20):
            acc.page.evaluate.return_value = {"status": 200, "body": "{}"}
            await self.host.handle({"cmd": "begin", "account": "account-a", "email": "fixture@example.invalid"})
            await self.host.handle({"cmd": "complete", "account": "account-a", "token": "fixture"})
            await self.host.handle({"cmd": "eval", "account": "account-a", "expr": "true"})
        self.assertEqual(acc.pool.qsize(), 1)
        self.assertEqual(len(acc.ctx.pages), 1)
        self.assertEqual(acc.active_requests, 0)
        self.assertFalse(acc.borrowed)

    async def test_failed_commands_return_their_page(self):
        acc = await self.account()
        for cmd in ("begin", "complete", "eval", "mint"):
            acc.page.evaluate.side_effect = RuntimeError("fixture failure")
            with patch.object(self.host, "_ensure_editor", AsyncMock(side_effect=RuntimeError("fixture failure"))):
                with self.assertRaises(RuntimeError):
                    await self.host.handle({"cmd": cmd, "account": "account-a",
                                            "email": "fixture", "token": "fixture", "expr": "true"})
            self.assertEqual(acc.pool.qsize(), 1)
            self.assertFalse(acc.borrowed)
            self.assertEqual(acc.active_requests, 0)

    async def test_cancellation_releases_page_and_activity(self):
        acc = await self.account()
        started = asyncio.Event()
        async def wait(*args):
            started.set()
            await asyncio.Future()
        acc.page.evaluate.side_effect = wait
        task = asyncio.create_task(self.host.handle({"cmd": "eval", "account": "account-a", "expr": "true"}))
        await started.wait()
        self.assertEqual(acc.active_requests, 1)
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertEqual(acc.active_requests, 0)
        self.assertEqual(acc.pool.qsize(), 1)

    async def test_concurrent_device_creation_is_single_flight(self):
        acc = await self.account()
        devices = await asyncio.gather(*(self.host._dev_page(acc, "account-a", "device") for _ in range(20)))
        self.assertEqual(len(acc.browser.contexts), 2)
        self.assertTrue(all(d is devices[0] for d in devices))
        devices[0].page.evaluate.assert_awaited_once()
        other = await self.account("account-b")
        other_device = await self.host._dev_page(other, "account-b", "device")
        self.assertIsNot(other_device.ctx, devices[0].ctx)

    async def test_device_failure_closes_partial_context_and_can_retry(self):
        acc = await self.account()
        failed = Context()
        failed.add_cookies.side_effect = RuntimeError("fixture failure")
        with patch.object(acc.browser, "new_context", AsyncMock(return_value=failed)):
            with self.assertRaises(RuntimeError):
                await self.host._dev_page(acc, "account-a", "device")
        self.assertTrue(failed.closed)
        self.assertIsNone(acc.dev_objs["device"].ctx)
        result = await self.host._dev_page(acc, "account-a", "device")
        self.assertIsNotNone(result.page)

    async def test_replacing_closed_page_closes_old_context(self):
        acc = await self.account()
        device = await self.host._dev_page(acc, "account-a", "device")
        old_ctx = device.ctx
        await device.page.close()
        await self.host._dev_page(acc, "account-a", "device")
        self.assertTrue(old_ctx.closed)
        self.assertIsNot(old_ctx, device.ctx)

    async def test_one_driver_keeps_distinct_account_browsers(self):
        driver = types.SimpleNamespace(stop=AsyncMock())
        manager = types.SimpleNamespace(start=AsyncMock(return_value=driver))
        launch = AsyncMock(side_effect=lambda *a, **kw: Browser())
        with patch.object(runtime, "async_playwright", Mock(return_value=manager)), \
             patch.object(runtime, "AsyncNewBrowser", launch):
            one = await self.host._account("account-a", "http://proxy-a.invalid")
            two = await self.host._account("account-b", "http://proxy-b.invalid")
        manager.start.assert_awaited_once()
        self.assertEqual(launch.await_count, 2)
        self.assertIsNot(one.browser, two.browser)
        self.assertIsNone(one.mint_page)
        self.assertIsNone(two.mint_page)
        self.assertNotEqual(launch.call_args_list[0].kwargs["proxy"],
                            launch.call_args_list[1].kwargs["proxy"])
        await self.host._close_account("account-a")
        self.assertFalse(two.browser.closed)
        driver.stop.assert_not_awaited()
        await self.host.close()
        driver.stop.assert_awaited_once()

    async def test_launch_failure_closes_browser(self):
        browser = Browser()
        manager = types.SimpleNamespace(start=AsyncMock(return_value=types.SimpleNamespace(stop=AsyncMock())))
        with patch.object(runtime, "async_playwright", Mock(return_value=manager)), \
             patch.object(runtime, "AsyncNewBrowser", AsyncMock(return_value=browser)), \
             patch.object(self.host, "_initialize_account", AsyncMock(side_effect=RuntimeError("fixture"))):
            with self.assertRaises(RuntimeError):
                await self.host._account("account-a")
        self.assertTrue(browser.closed)
        self.assertNotIn("account-a", self.host.accounts)

    async def test_idle_shutdown_never_closes_active_ai_request(self):
        acc = await self.account()
        self.host.idle_timeout = 1
        started, finish = asyncio.Event(), asyncio.Event()
        async def work(req):
            started.set()
            await finish.wait()
        with patch.object(self.host, "_handle", work):
            task = asyncio.create_task(self.host.handle({"cmd": "ai_post", "account": "account-a"}))
            await started.wait()
            acc.last_used = time.monotonic() - 100
            await self.host._monitor_once()
            self.assertFalse(acc.browser.closed)
            with self.assertRaisesRegex(RuntimeError, "active requests"):
                await self.host.handle({"cmd": "close", "account": "account-a"})
            finish.set()
            await task
        await self.host._monitor_once()
        self.assertFalse(acc.browser.closed)
        acc.last_used = time.monotonic() - 100
        await self.host._monitor_once()
        self.assertTrue(acc.browser.closed)

    async def test_default_keeps_idle_accounts_warm(self):
        acc = await self.account()
        acc.last_used = 0
        await self.host._monitor_once()
        self.assertFalse(acc.browser.closed)

    async def test_account_close_cancels_its_background_tasks(self):
        acc = await self.account()
        started, stopped = asyncio.Event(), asyncio.Event()
        async def background():
            started.set()
            try:
                await asyncio.Future()
            finally:
                stopped.set()
        task = self.host._track_task(acc, background())
        await started.wait()
        await self.host._close_account("account-a")
        self.assertTrue(stopped.is_set())
        self.assertTrue(task.done())
        self.assertFalse(acc.tasks)

    async def test_job_timeout_aborts_browser_fetch(self):
        acc = await self.account()
        async def evaluate(script, arg):
            if script == runtime.AI_POLL_JS:
                await asyncio.Future()
        acc.page.evaluate.side_effect = evaluate
        result = await self.host._run_job(acc.page, "/fixture", {}, "fixture", None, "job", None, 0.01)
        self.assertEqual(result["status"], -1)
        self.assertEqual(acc.page.evaluate.await_args_list[-1].args, (runtime.AI_CANCEL_JS, "job"))

    async def test_job_cancellation_aborts_browser_fetch(self):
        acc = await self.account()
        started = asyncio.Event()
        async def evaluate(script, arg):
            if script == runtime.AI_POLL_JS:
                started.set()
                await asyncio.Future()
        acc.page.evaluate.side_effect = evaluate
        task = asyncio.create_task(self.host._run_job(acc.page, "/fixture", {}, "fixture", None, "job", None, 30))
        await started.wait()
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertEqual(acc.page.evaluate.await_args_list[-1].args, (runtime.AI_CANCEL_JS, "job"))

    async def test_device_request_does_not_load_unused_editor(self):
        acc = await self.account()
        acc.devices = ["device"]
        acc.primary_dev = "device"
        device = types.SimpleNamespace(page=Page())
        with patch.object(self.host, "_dev_page", AsyncMock(return_value=device)), \
             patch.object(self.host, "_token_acquire", AsyncMock(return_value=(None, "fixture", "device"))), \
             patch.object(self.host, "_run_job", AsyncMock(return_value={"status": 200, "body": "{}"})), \
             patch.object(self.host, "_ensure_editor", AsyncMock()) as editor:
            result = await self.host.handle({"cmd": "ai_post", "account": "account-a",
                                             "path": "/fixture", "body": {"projectId": "fixture"}})
        self.assertEqual(result["status"], 200)
        editor.assert_not_awaited()

    async def test_allocation_reservations_block_concurrent_overcommit(self):
        with patch.object(runtime, "memory_snapshot", Mock(return_value={"allocation_headroom_bytes": 100})):
            async with self.host._allocation(60):
                with self.assertRaises(runtime.MemoryPressureError):
                    async with self.host._allocation(60):
                        self.fail("allocation should have been rejected")
            self.assertEqual(self.host._reserved_bytes, 0)
            async with self.host._allocation(60):
                pass

    async def test_eof_stops_driver_and_all_browsers(self):
        acc = await self.account()
        driver = types.SimpleNamespace(stop=AsyncMock())
        self.host._playwright = driver
        with patch.object(sys, "stdin", io.StringIO("")):
            await self.host.loop()
        self.assertTrue(acc.browser.closed)
        driver.stop.assert_awaited_once()

    async def test_closed_borrowed_page_wakes_waiter(self):
        acc = await self.account()
        with patch.object(runtime, "POOL_CAP", 1):
            page, _, _ = await self.host._acquire("account-a")
            waiter = asyncio.create_task(self.host._acquire("account-a"))
            await asyncio.sleep(0)
            await page.close()
            self.host._release(acc, page)
            replacement, _, _ = await asyncio.wait_for(waiter, 1)
            self.assertIsNot(replacement, page)
            self.host._release(acc, replacement)
        self.assertEqual(acc.size, 1)
        self.assertEqual(acc.pool.qsize(), 1)


    async def test_cancel_protocol_releases_active_request(self):
        acc = await self.account()
        lines = queue.Queue()
        reader = types.SimpleNamespace(readline=lines.get)
        started, cancelled = asyncio.Event(), asyncio.Event()
        async def evaluate(*args):
            started.set()
            try:
                await asyncio.Future()
            finally:
                cancelled.set()
        acc.page.evaluate.side_effect = evaluate
        with patch.object(sys, "stdin", reader), patch.object(sys, "stdout", io.StringIO()):
            task = asyncio.create_task(self.host.loop())
            try:
                lines.put('{"id":1,"cmd":"eval","account":"account-a","expr":"true"}\n')
                await asyncio.wait_for(started.wait(), 2)
                lines.put('{"cmd":"cancel","request_id":1}\n')
                await asyncio.wait_for(cancelled.wait(), 2)
            finally:
                lines.put("")
                await asyncio.wait_for(task, 2)
        self.assertEqual(acc.active_requests, 0)
        self.assertFalse(acc.borrowed)


class MemorySnapshotTests(unittest.TestCase):
    def test_tightest_ancestor_budget_wins(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            proc, cg = root / "proc", root / "cgroup"
            (proc / "self").mkdir(parents=True)
            (proc / "meminfo").write_text("MemTotal: 10000 kB\nMemAvailable: 9000 kB\n")
            (proc / "self/cgroup").write_text("0::/parent/child\n")
            for path, used, maximum in ((cg, 9000, "max"), (cg / "parent", 8500, "10000"),
                                        (cg / "parent/child", 1000, "2000")):
                path.mkdir(parents=True, exist_ok=True)
                (path / "memory.current").write_text(str(used))
                (path / "memory.max").write_text(maximum)
            snapshot = runtime.memory_snapshot(proc, cg)
            self.assertEqual(snapshot["allocation_headroom_bytes"], 500)
            self.assertEqual(snapshot["cgroup_current_bytes"], 1000)

    def test_missing_proc_is_supported(self):
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(runtime.memory_snapshot(Path(tmp), Path(tmp)), {})
