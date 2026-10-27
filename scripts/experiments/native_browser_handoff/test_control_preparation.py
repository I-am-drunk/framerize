"""Real host launch/close ownership with inert driver, browser and page ports."""
import asyncio
from dataclasses import replace
import json
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "runtime"))
sys.path.insert(0, str(ROOT / "runtime/tests"))
from framerize_host import app
from framerize_host.browser import driver
from test_lifecycle import FakeBrowser
from test_control_mint import EXPECTED, Page
from control_owner import Consumed, ControlIdentity, _Binding
from control_preparation import PreparedControlHost, ObservationRefused

ACCOUNT = "00000000-0000-0000-0000-000000000001"


class Ports:
    def __init__(self):
        self.calls, self.entered, self.release, self.errors = [], {}, {}, set()
        self.browsers, self.kwargs = [], []
        self.stopped, self.redirect = False, None

    def hold(self, name):
        self.entered[name], self.release[name] = asyncio.Event(), asyncio.Event()

    async def step(self, name):
        self.calls.append(name)
        if name in self.entered:
            self.entered[name].set()
            await self.release[name].wait()
        if name in self.errors:
            raise RuntimeError("fixture-private-detail")

    async def start(self):
        await self.step("driver-start")
        return self

    async def stop(self):
        await self.step("driver-stop")
        self.stopped = True

    async def launch(self, playwright, **kwargs):
        assert playwright is self
        self.kwargs.append(kwargs)
        await self.step("launch")
        browser = Browser(self)
        self.browsers.append(browser)
        return browser


class Browser(FakeBrowser):
    def __init__(self, ports):
        super().__init__()
        self.ports = ports

    async def new_context(self):
        await self.ports.step("context")
        context = Context(self)
        self.contexts.append(context)
        return context

    def disconnect(self):
        for context in self.contexts:
            for page in context.pages:
                page.closed = True
        super().disconnect()


class Context:
    def __init__(self, browser):
        self.browser, self.ports = browser, browser.ports
        self.scripts, self.values, self.pages = [], [], []

    async def add_init_script(self, script):
        await self.ports.step("device-script")
        self.scripts.append(script)

    async def add_cookies(self, values):
        await self.ports.step("install-cookies")
        self.values = [dict(value) for value in values]

    async def new_page(self):
        await self.ports.step("page")
        page = PreparedPage(self)
        self.pages.append(page)
        return page

    async def cookies(self, url):
        await self.ports.step("read-cookies")
        return [dict(value) for value in self.values]


class PreparedPage(Page):
    def __init__(self, context):
        super().__init__()
        self.context, self.url, self.navigation = context, "about:blank", None

    async def goto(self, url, **kwargs):
        self.navigation = dict(url=url, **kwargs)
        assert {c["name"] for c in self.context.values} == {"session", "__cuid"}
        assert len(self.context.scripts) == 1
        await self.context.ports.step("navigate")
        self.url = self.context.ports.redirect or url


class PreparationTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.ports = Ports()
        overrides = patch.multiple(app, ROOT=directory.name,
            emit=lambda *a, **k: None, trace=lambda *a, **k: None,
            browser_runtime=lambda: (self.ports.launch, lambda: self.ports),
            prepared_memory_browser=lambda: None, prepared_memory_addon=lambda: None,
            memory_snapshot=lambda: {"allocation_headroom_bytes": 4 * 1024 ** 3})
        overrides.start()
        self.addCleanup(overrides.stop)
        protocol = patch.object(driver, "verify_close_protocol", lambda: None)
        protocol.start()
        self.addCleanup(protocol.stop)
        environment = patch.dict("os.environ", {"FRAMERIZE_PROXY": ""})
        environment.start()
        self.addCleanup(environment.stop)
        self.host = PreparedControlHost()
        self.tasks, self.allow_uncertain = [], False

    async def start(self, **overrides):
        args = dict(account=ACCOUNT, expected=EXPECTED, proxy=None,
            proxy_revision=7, deadline=time.monotonic() + 5)
        args.update(overrides)
        return await self.host.prepare(**args)

    def spawn(self, coroutine):
        task = asyncio.create_task(coroutine)
        self.tasks.append(task)
        return task

    async def entered(self, name):
        await asyncio.wait_for(self.ports.entered[name].wait(), 1)

    async def asyncTearDown(self):
        for event in self.ports.release.values():
            event.set()
        for browser in self.ports.browsers:
            browser.release.set()
        for task in self.tasks:
            if not task.done():
                task.cancel()
        await asyncio.gather(*self.tasks, return_exceptions=True)
        result = await asyncio.gather(self.host.close(), return_exceptions=True)
        if not self.allow_uncertain:
            self.assertEqual(result, [None])
        for life in self.host.lifecycles.values():
            self.assertFalse(life.activity.pending or life.activity.activities)

    async def consume(self, minted, cancelled, deadline):
        self.assertEqual(minted.token, "fixture-token")
        return Consumed("complete", True)

    async def test_preparation_binds_one_page_without_mint_or_background_work(self):
        identity = await self.start()
        acc = self.host.accounts[ACCOUNT]
        page = acc.dev_objs[EXPECTED.device].page
        self.assertEqual(identity, ControlIdentity(ACCOUNT, self.host.generation, 1, 7))
        self.assertTrue(_Binding(self.host, identity, EXPECTED).current())
        self.assertEqual(self.ports.calls, ["driver-start", "launch", "context",
            "device-script", "install-cookies", "page", "navigate"])
        self.assertEqual(page.navigation["url"], EXPECTED.page_url)
        self.assertEqual(page.navigation["wait_until"], "domcontentloaded")
        self.assertGreater(page.navigation["timeout"], 0)
        self.assertLessEqual(page.navigation["timeout"], 5000)
        self.assertEqual({c["name"]: c["value"] for c in acc.ctx.values},
            {"session": EXPECTED.session, "__cuid": EXPECTED.device})
        self.assertIn(json.dumps(EXPECTED.device), acc.ctx.scripts[0])
        self.assertEqual(page.calls, [])
        self.assertEqual(acc.devices, [])
        self.assertFalse(acc.tasks or acc.token_pool or acc.devpool_ready)
        self.assertTrue(acc.pool.empty())
        self.assertIsNone(self.host.rate_journal)
        self.assertFalse(acc.lifecycle.activity.pending or acc.lifecycle.activity.activities)
        await self.host.close()
        self.assertTrue(self.ports.stopped and page.closed)
        self.assertEqual(self.ports.browsers[0].calls, 1)

    async def test_prepared_page_uses_existing_control_and_confirmed_cleanup(self):
        identity = await self.start()
        page = self.host.accounts[ACCOUNT].page
        result = await self.host.run_control(identity, EXPECTED, self.consume,
            time.monotonic() + 5)
        self.assertEqual(result, Consumed("complete", True))
        self.assertEqual(page.calls, ["evaluate"])
        self.assertEqual(self.ports.calls.count("read-cookies"), 4)
        with self.assertRaises(ObservationRefused):
            await self.host.run_control(identity, EXPECTED, self.consume,
                time.monotonic() + 5)
        await self.host.close()
        self.assertTrue(self.ports.stopped)
        self.assertFalse(self.host.accounts)

    async def test_ordinary_commands_and_preparation_reuse_refuse(self):
        await self.start()
        with self.assertRaises(ObservationRefused):
            await self.start()
        with self.assertRaises(ObservationRefused):
            await self.host.handle(dict(cmd="mint", account=ACCOUNT))
        with self.assertRaises(ObservationRefused):
            await self.host._rpc(ACCOUNT, self.ports.step, "unexpected")
        with self.assertRaises(ObservationRefused):
            async with self.host._activity(ACCOUNT):
                self.fail("ordinary activity entered")
        self.assertNotIn("unexpected", self.ports.calls)

    async def test_invalid_identity_deadline_and_proxy_refuse_before_launch(self):
        invalid = [dict(account="../outside"), dict(account="/tmp/outside"),
            dict(account=ACCOUNT.replace("-", "")), dict(proxy_revision=True),
            dict(proxy_revision=-1), dict(proxy_revision=2 ** 64), dict(proxy=""),
            dict(expected=replace(EXPECTED, session="")), dict(deadline=0),
            dict(deadline=float("inf")), dict(deadline=time.monotonic() + 181)]
        for changes in invalid:
            with self.subTest(fields=list(changes)):
                with self.assertRaises(ObservationRefused) as caught:
                    await self.start(**changes)
                self.assertEqual(str(caught.exception), "browser mint observation refused")
                self.assertEqual(self.ports.calls, [])
                self.assertEqual(self.host.lifecycles, {})

    async def test_direct_route_refuses_implicit_environment_proxy(self):
        with patch.dict("os.environ", {"FRAMERIZE_PROXY": "http://fixture.invalid:9"}):
            with self.assertRaises(ObservationRefused):
                await self.start()
        self.assertEqual(self.ports.calls, [])

    async def test_explicit_proxy_is_bound_to_launch_and_revision(self):
        proxy = "http://fixture.invalid:8080"
        identity = await self.start(proxy=proxy)
        self.assertEqual(identity.proxy_revision, 7)
        self.assertEqual(self.ports.kwargs[0]["proxy"]["server"], proxy)
        self.assertEqual(self.host.proxies, {ACCOUNT: proxy})

    async def assert_cancelled_at(self, stage):
        self.ports.hold(stage)
        task = self.spawn(self.start())
        await self.entered(stage)
        before = list(self.ports.calls)
        task.cancel()
        await asyncio.wait_for(self.host._preparation_stopped.wait(), 1)
        self.assertFalse(task.done())
        self.assertTrue(self.host.lifecycles[ACCOUNT].activity.pending)
        self.ports.release[stage].set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(asyncio.shield(task), 2)
        self.assertEqual(self.ports.calls, before + ["driver-stop"])
        self.assertTrue(self.ports.stopped)
        self.assertFalse(self.host.accounts)
        self.assertEqual(self.host.lifecycles[ACCOUNT].phase, "absent")
        for browser in self.ports.browsers:
            self.assertFalse(browser.is_connected())
            self.assertEqual(browser.calls, 1)

    async def test_cancelled_driver_start_is_joined_and_stopped(self):
        await self.assert_cancelled_at("driver-start")

    async def test_cancelled_launch_keeps_late_browser_owned(self):
        await self.assert_cancelled_at("launch")

    async def test_cancelled_context_creation_closes_late_context(self):
        await self.assert_cancelled_at("context")

    async def test_cancelled_cookie_write_cannot_start_page(self):
        await self.assert_cancelled_at("install-cookies")

    async def test_cancelled_page_creation_cannot_navigate(self):
        await self.assert_cancelled_at("page")

    async def test_cancelled_navigation_joins_then_closes(self):
        await self.assert_cancelled_at("navigate")

    async def test_preparation_deadline_joins_late_page_without_navigation(self):
        self.ports.hold("page")
        task = self.spawn(self.start(deadline=time.monotonic() + 0.15))
        await self.entered("page")
        await asyncio.wait_for(self.host._preparation_stopped.wait(), 1)
        self.assertFalse(task.done())
        self.ports.release["page"].set()
        with self.assertRaises(ObservationRefused):
            await asyncio.wait_for(asyncio.shield(task), 2)
        self.assertNotIn("navigate", self.ports.calls)
        self.assertTrue(self.ports.stopped)
        self.assertFalse(self.host.accounts)

    async def test_second_cancellation_cannot_abandon_close_acknowledgement(self):
        self.ports.hold("navigate")
        task = self.spawn(self.start())
        await self.entered("navigate")
        browser = self.ports.browsers[0]
        browser.release.clear()
        task.cancel()
        await asyncio.wait_for(self.host._preparation_stopped.wait(), 1)
        self.ports.release["navigate"].set()
        await asyncio.wait_for(browser.called.wait(), 1)
        task.cancel()
        await asyncio.sleep(0)
        self.assertFalse(task.done())
        self.assertFalse(self.ports.stopped)
        browser.release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(asyncio.shield(task), 2)
        self.assertTrue(self.ports.stopped)
        self.assertEqual(browser.calls, 1)

    async def test_concurrent_shutdown_joins_preparation_before_driver_stop(self):
        self.ports.hold("context")
        preparing = self.spawn(self.start())
        await self.entered("context")
        closing = self.spawn(self.host.close())
        await asyncio.wait_for(self.host._preparation_stopped.wait(), 1)
        self.assertFalse(closing.done())
        self.ports.release["context"].set()
        with self.assertRaises(ObservationRefused):
            await asyncio.wait_for(asyncio.shield(preparing), 2)
        await closing
        self.assertTrue(self.ports.stopped)

    async def test_cookie_installation_error_closes_without_navigation(self):
        self.ports.errors.add("install-cookies")
        with self.assertRaises(ObservationRefused) as caught:
            await self.start()
        self.assertNotIn("fixture-private-detail", str(caught.exception))
        self.assertNotIn("page", self.ports.calls)
        self.assertNotIn("navigate", self.ports.calls)
        self.assertTrue(self.ports.stopped)
        self.assertEqual(self.ports.browsers[0].calls, 1)

    async def test_redirected_page_refuses_and_closes_before_mint(self):
        self.ports.redirect = "https://framer.com/login"
        with self.assertRaises(ObservationRefused):
            await self.start()
        page = self.ports.browsers[0].contexts[0].pages[0]
        self.assertEqual(page.calls, [])
        self.assertTrue(page.closed and self.ports.stopped)

    async def test_proxy_revision_change_during_preparation_refuses(self):
        self.ports.hold("context")
        task = self.spawn(self.start())
        await self.entered("context")
        self.host.proxy_revisions[ACCOUNT] += 1
        self.ports.release["context"].set()
        with self.assertRaises(ObservationRefused):
            await task
        self.assertNotIn("device-script", self.ports.calls)
        self.assertTrue(self.ports.stopped)

    async def test_changed_expected_identity_cannot_use_prepared_page(self):
        identity = await self.start()
        for expected in (replace(EXPECTED, session="different"),
                replace(EXPECTED, user="different"),
                replace(EXPECTED, device="different")):
            with self.subTest(field=repr(expected)):
                with self.assertRaises(ObservationRefused):
                    await self.host.run_control(identity, expected, self.consume,
                        time.monotonic() + 5)
        self.assertFalse(self.host._control_used)
        self.assertEqual(self.host.accounts[ACCOUNT].page.calls, [])

    async def test_lost_close_acknowledgement_remains_unconfirmed(self):
        await self.start()
        self.allow_uncertain = True
        browser = self.ports.browsers[0]
        browser.failure = True
        with self.assertRaises(RuntimeError):
            await self.host.close()
        self.assertEqual(self.host.lifecycles[ACCOUNT].phase, "unconfirmed")
        self.assertTrue(browser.is_connected())
        self.assertFalse(self.ports.stopped)
        with self.assertRaises(ObservationRefused):
            await self.start()


if __name__ == "__main__":
    unittest.main()
