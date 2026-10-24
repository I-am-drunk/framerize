"""Inert source fixtures. No browser, SDK, account store or network imports."""
import ast
import asyncio
import os
import sys
import tempfile
import time
import types
import unittest
import uuid
from contextlib import asynccontextmanager
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))


class Events:
    def __init__(self):
        self.listeners = {}

    def on(self, name, callback):
        self.listeners.setdefault(name, []).append(callback)

    def remove_listener(self, name, callback):
        self.listeners[name].remove(callback)

    def fire(self, name):
        for callback in tuple(self.listeners.get(name, ())):
            callback()


class Browser(Events):
    def __init__(self):
        super().__init__()
        self.contexts, self.connected = [], True

    def is_connected(self):
        return self.connected


class Context(Events):
    def __init__(self, browser):
        super().__init__()
        self.browser, self.pages = browser, []
        browser.contexts.append(self)


class Page(Events):
    def __init__(self, context):
        super().__init__()
        self.context, self.closed = context, False
        context.pages.append(self)

    def is_closed(self):
        return self.closed

    async def evaluate(self, *args):
        raise AssertionError("provenance must not issue page RPCs")


class ResidentHttp200Tests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.events, self.sends, self.mints, self.releases = [], [], [], []
        source = Path(__file__).resolve().parents[2] / "scripts/camoufox_host.py"
        names = {"ResidentBrowserHttp200", "AccountLifecycle", "Account", "Host"}
        selected = [n for n in ast.parse(source.read_text()).body
                    if isinstance(n, ast.ClassDef) and n.name in names]
        self.assertEqual({n.name for n in selected}, names)
        self.ns = dict(asyncio=asyncio, os=os, time=time, uuid=uuid, Path=Path,
            asynccontextmanager=asynccontextmanager, ROOT="unused-fixture-root",
            trace=lambda *a, **k: None, emit=lambda *a, **k: self.events.append((a, k)))
        exec(compile(ast.Module(body=selected, type_ignores=[]), str(source), "exec"), self.ns)
        self.host = self.ns["Host"].__new__(self.ns["Host"])
        self.host.accounts, self.host.lifecycles = {}, {}
        self.host.generation, self.host._observed = "fixture-host", None
        self.acc = self.make_account("account")
        self.device = types.SimpleNamespace(id="device", ctx=Context(self.acc.browser))
        self.device.page = Page(self.device.ctx)
        self.acc.dev_objs["device"] = self.device
        self.acc.devices = ["device"]
        self.choice = "device"
        self.entered, self.release = asyncio.Event(), asyncio.Event()
        self.release.set()
        self.result = {"status": 200, "usage": None, "body": "not parsed inference JSON"}
        async def account(*args):
            return self.acc
        async def device(*args):
            return self.device
        async def mint(*args):
            self.mints.append(args)
            return "slot", "synthetic-token-secret", self.choice
        async def send(*args):
            self.sends.append(args)
            self.entered.set()
            await self.release.wait()
            return self.result
        async def ensure(*args):
            pass
        self.host._account, self.host._dev_page = account, device
        self.host._token_acquire, self.host._run_job = mint, send
        self.host._ensure_editor = ensure
        self.host._set_editor = lambda *args: None
        self.host._pick_device_sticky = lambda *args: self.choice
        self.host._pick_device = lambda *args: self.choice
        self.host._note_request = lambda *args: None
        self.host._token_release = lambda *args: self.releases.append(args)

    def request(self):
        return dict(cmd="ai_post", account="account", path="/ai/v3/responses/",
            bearer="synthetic-bearer-secret", defer_retries=True,
            body={"projectId": "project", "request": {"model": "fixture"}})

    def make_account(self, name):
        life = self.ns["AccountLifecycle"]()
        life.phase, life.incarnation, life.browser = "resident", 7, Browser()
        context = Context(life.browser)
        acc = self.ns["Account"](None, context, Page(context), life)
        acc.browser, acc.project = life.browser, "project"
        acc.resident_launch = (life.browser, 7, "http://fixture-proxy", False)
        self.host.accounts[name], self.host.lifecycles[name] = acc, life
        return acc

    def candidate(self):
        return self.host._capture_browser_http200(
            "account", self.acc, self.device.page, "project")

    async def launch_fixture(self, account_proxy=None, global_proxy=None, comparison=False):
        host, seen = self.host, {}
        host.proxies = {"account": account_proxy} if account_proxy else {}
        host._playwright, host._memory_browser = object(), None
        host._driver_start_task = None
        host._memory_addon = {"path": "synthetic-addon", "excluded_default": "synthetic"}
        scope = types.SimpleNamespace(comparison_phase="context_pending",
            begin_launch=lambda: None, browser_started=lambda browser: object())
        host._observed = scope if comparison else None
        host._observed_scope = lambda account: host._observed
        async def browser_factory(playwright, **kwargs):
            seen.update(kwargs)
            host.proxies["account"] = "http://changed-after-launch"
            os.environ["FRAMERIZE_PROXY"] = "http://changed-environment"
            return Browser()
        async def initialize(account, profile, browser):
            ctx = Context(browser)
            acc = self.ns["Account"](None, ctx, Page(ctx), host.lifecycles[account])
            acc.browser = browser
            host.accounts[account] = acc
            return acc
        @asynccontextmanager
        async def allocation(estimate):
            yield
        host._browser_runtime = (browser_factory, None)
        host._initialize_account, host._allocation = initialize, allocation
        self.ns.update(BROWSER_STARTUP_RESERVE=1, BrowserCloseReceipt=lambda browser: object())
        with tempfile.TemporaryDirectory(prefix="resident-launch-fixture-") as root:
            self.ns["ROOT"] = root
            with patch.dict(os.environ, {"FRAMERIZE_PROXY": global_proxy or ""}):
                result = await host._launch("account")
        return result, seen

    async def test_launch_proxy_captures_actual_account_override(self):
        acc, seen = await self.launch_fixture("http://account-proxy", "http://global-proxy")
        self.assertEqual(seen["proxy"], {"server": "http://account-proxy"})
        self.assertEqual(acc.resident_launch, (acc.browser, 7, "http://account-proxy", False))

    async def test_launch_proxy_captures_global_fallback(self):
        acc, seen = await self.launch_fixture(global_proxy="http://global-proxy")
        self.assertEqual(seen["proxy"], {"server": "http://global-proxy"})
        self.assertEqual(acc.resident_launch[2:], ("http://global-proxy", False))

    async def test_direct_launch_is_distinct_from_unknown_capability(self):
        self.assertIsNone(self.ns["Account"](None, None, None).resident_launch)
        acc, seen = await self.launch_fixture()
        self.assertNotIn("proxy", seen)
        self.assertEqual(acc.resident_launch[2:], (None, False))

    async def test_comparison_launch_records_existing_realm_opt_in(self):
        acc, seen = await self.launch_fixture(global_proxy="http://unused-global", comparison=True)
        self.assertNotIn("proxy", seen)
        self.assertIs(seen["main_world_eval"], True)
        self.assertEqual(acc.resident_launch[2:], (None, True))

    async def test_completed_http200_records_actual_presend_tuple(self):
        self.release.clear()
        task = asyncio.create_task(self.host._handle(self.request()))
        try:
            await asyncio.wait_for(self.entered.wait(), 1)
            self.assertIsNone(self.acc.completed_browser_http200)
        finally:
            self.release.set()
        result = await asyncio.wait_for(task, 1)
        evidence = self.acc.completed_browser_http200
        self.assertIs(evidence.page, self.sends[0][0])
        self.assertIs(evidence.page, self.device.page)
        self.assertIs(evidence.context, self.device.ctx)
        self.assertIs(evidence.browser, self.acc.browser)
        self.assertEqual((evidence.project, evidence.incarnation), ("project", 7))
        self.assertIs(evidence.launch, self.acc.resident_launch)
        self.assertIs(result, self.result)
        self.assertEqual(set(result), {"status", "usage", "body", "_fetch_ms", "queue_wait_ms"})
        self.assertEqual((len(self.sends), len(self.mints), len(self.releases)), (1, 1, 1))
        self.assertEqual(result["body"], "not parsed inference JSON")
        public = repr(self.events) + repr(evidence)
        for secret in ("synthetic-token-secret", "synthetic-bearer-secret", "fixture-proxy"):
            self.assertNotIn(secret, public)

    async def test_ambient_http200_records_actual_ambient_context(self):
        self.choice, self.acc.devices = None, []
        await self.host._handle(self.request())
        evidence = self.acc.completed_browser_http200
        self.assertIs(evidence.page, self.acc.page)
        self.assertIs(evidence.context, self.acc.ctx)
        self.assertIs(evidence.owner, self.acc)
        self.assertEqual((len(self.sends), len(self.mints)), (1, 1))

    def test_missing_launch_or_unowned_page_cannot_supply_provenance(self):
        unowned = Page(Context(self.acc.browser))
        self.assertIsNone(self.host._capture_browser_http200(
            "account", self.acc, unowned, "project"))
        self.acc.resident_launch = None
        self.assertIsNone(self.candidate())

    def test_only_integer_http200_is_recorded(self):
        for status in (None, True, "200", 200.0, -1, 201, 403):
            with self.subTest(status=status):
                self.host._record_browser_http200(self.acc, self.candidate(), {"status": status})
                self.assertIsNone(self.acc.completed_browser_http200)

    def test_page_context_and_browser_close_clear_provenance(self):
        for source, event in ((self.device.page, "close"), (self.device.ctx, "close"),
                              (self.acc.browser, "disconnected")):
            with self.subTest(event=event):
                self.host._record_browser_http200(self.acc, self.candidate(), {"status": 200})
                self.assertIsNotNone(self.acc.completed_browser_http200)
                source.fire(event)
                self.assertIsNone(self.acc._completed_browser_http200)

    def test_replacement_clears_record_when_read(self):
        self.host._record_browser_http200(self.acc, self.candidate(), {"status": 200})
        self.device.page = Page(self.device.ctx)
        self.assertIsNone(self.acc.completed_browser_http200)
        self.assertIsNone(self.acc._completed_browser_http200)

    def test_new_completion_detaches_old_watchers_and_other_account_survives(self):
        other = self.make_account("other")
        other_evidence = self.host._capture_browser_http200("other", other, other.page, "project")
        self.host._record_browser_http200(other, other_evidence, {"status": 200})
        old = self.candidate()
        self.host._record_browser_http200(self.acc, old, {"status": 200})
        self.host._record_browser_http200(self.acc, self.candidate(), {"status": 200})
        self.assertTrue(old.invalid)
        self.assertEqual(old.listeners, [])
        self.device.ctx.fire("close")
        self.assertIs(other.completed_browser_http200, other_evidence)

    async def test_replaced_page_while_send_is_pending_does_not_publish(self):
        self.release.clear()
        task = asyncio.create_task(self.host._handle(self.request()))
        try:
            await asyncio.wait_for(self.entered.wait(), 1)
            self.device.page = Page(self.device.ctx)
        finally:
            self.release.set()
        self.assertEqual((await asyncio.wait_for(task, 1))["status"], 200)
        self.assertIsNone(self.acc.completed_browser_http200)
        self.assertEqual((len(self.sends), len(self.mints)), (1, 1))

    async def test_cancelled_send_does_not_publish(self):
        self.release.clear()
        task = asyncio.create_task(self.host._handle(self.request()))
        try:
            await asyncio.wait_for(self.entered.wait(), 1)
        finally:
            task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertIsNone(self.acc.completed_browser_http200)
        self.assertEqual(len(self.releases), 1)

    async def test_observation_failure_does_not_change_send(self):
        def unavailable(*args):
            raise RuntimeError("synthetic-private-error")
        self.device.page.on = unavailable
        result = await self.host._handle(self.request())
        self.assertEqual(result["status"], 200)
        self.assertIsNone(self.acc.completed_browser_http200)
        self.assertEqual((len(self.sends), len(self.mints), len(self.releases)), (1, 1, 1))
        self.assertNotIn("synthetic-private-error", repr(self.events))

    async def test_send_failure_propagates_without_provenance(self):
        async def fail(*args):
            raise RuntimeError("synthetic send failure")
        self.host._run_job = fail
        with self.assertRaisesRegex(RuntimeError, "synthetic send failure"):
            await self.host._handle(self.request())
        self.assertIsNone(self.acc.completed_browser_http200)
        self.assertEqual(len(self.releases), 1)

    def test_changed_bound_identities_never_publish(self):
        mutations = {
            "account": lambda: self.host.accounts.pop("account"),
            "lifecycle": lambda: self.host.lifecycles.pop("account"),
            "generation": lambda: setattr(self.host, "generation", "other"),
            "incarnation": lambda: setattr(self.acc.lifecycle, "incarnation", 8),
            "browser": lambda: setattr(self.acc, "browser", Browser()),
            "context": lambda: setattr(self.device, "ctx", Context(self.acc.browser)),
            "project": lambda: setattr(self.acc, "project", "other"),
            "registry": lambda: self.acc.dev_objs.clear(),
            "launch": lambda: setattr(self.acc, "resident_launch", None),
        }
        for name, mutate in mutations.items():
            with self.subTest(identity=name):
                self.host.generation = "fixture-host"
                self.acc = self.make_account("account")
                self.device = types.SimpleNamespace(ctx=Context(self.acc.browser))
                self.device.page = Page(self.device.ctx)
                self.acc.dev_objs["device"] = self.device
                candidate = self.candidate()
                self.assertIsNotNone(candidate)
                mutate()
                self.host._record_browser_http200(self.acc, candidate, {"status": 200})
                self.assertIsNone(self.acc.completed_browser_http200)


if __name__ == "__main__":
    unittest.main()
