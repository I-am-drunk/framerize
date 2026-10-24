"""Offline observed/pool seams; real host classes, fake browser operations."""
import ast
import asyncio
import os
import sys
import tempfile
import time
import types
import unittest
import uuid
from collections import deque
from contextlib import asynccontextmanager
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
from contextual_authorization_tests import FakeContext, FakePage, LOGIN, PROJECT
from observed_sidecar_tests import FakeWriter, obs
from resident_lifecycle_tests import FakeBrowser


class ObserverPoolLifecycleTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="observer-pool-")
        self.root = Path(self.directory.name)
        self.addCleanup(self.directory.cleanup)
        self.nonce, self.incarnation = str(uuid.uuid4()), str(uuid.uuid4())
        (self.root / ".observer-owner").write_text(self.nonce)
        self.browsers, self.launches, self.seen, self.gates = [], [], [], []
        self.sequence = self.driver_starts = self.driver_stops = 0
        self.ns = dict(asyncio=asyncio, os=os, uuid=uuid, time=time, Path=Path,
            asynccontextmanager=asynccontextmanager, ROOT=str(self.root),
            prepared_memory_browser=lambda: None, prepared_memory_addon=lambda: None,
            trace=lambda *args, **kwargs: None, emit=lambda *args, **kwargs: None,
            verify_close_protocol=lambda: None, BROWSER_STARTUP_RESERVE=1024,
            memory_snapshot=lambda: {"allocation_headroom_bytes": 1048576},
            MemoryPressureError=RuntimeError, DEVICE_POOL=1, RESERVE_PER_DAY=0,
            async_playwright=lambda: types.SimpleNamespace(start=self.start_driver),
            AsyncNewBrowser=self.new_browser, CASTLE_CHUNK="https://framer.com/fixture.js")
        self.ns["browser_runtime"] = lambda: (
            self.ns["AsyncNewBrowser"], self.ns["async_playwright"])
        source = self.root_source()
        names = {"BrowserCloseReceipt", "AccountLifecycle", "Account", "Host"}
        selected = [n for n in ast.parse(source.read_text()).body
                    if isinstance(n, ast.ClassDef) and n.name in names]
        self.assertEqual({n.name for n in selected}, names)
        exec(compile(ast.Module(body=selected, type_ignores=[]), str(source), "exec"), self.ns)
        self.environment = patch.dict(os.environ, {
            "FRAMERIZE_SIDECAR_MODE": obs.MODE,
            "FRAMERIZE_SIDECAR_INCARNATION": self.incarnation,
            "FRAMERIZE_OBSERVED_PROFILE_NONCE": self.nonce,
            "FRAMERIZE_PROXY": "http://ambient.invalid:9999",
            "FRAMERIZE_IDLE_TIMEOUT_S": "0", "FRAMERIZE_IDLE_GC_S": "120"})
        self.environment.start()
        self.addCleanup(self.environment.stop)
        self.host = self.ns["Host"]()
        self.owner, self.writer = self.host._observed, FakeWriter()
        self.owner.writer = self.writer
        self.real_initialize = self.host._initialize_account
        self.host._initialize_account = self.initialize
        self.host._handle = self.handle_account
        self.protocol = patch.object(obs, "verify_close_protocol", return_value=None)
        self.protocol.start()
        self.addCleanup(self.protocol.stop)
        hello = await self.reply(self.send("observer_hello", scopes=4, commands=8))
        self.assertTrue(hello["ok"])

    @staticmethod
    def root_source():
        return Path(__file__).resolve().parents[2] / "scripts" / "camoufox_host.py"

    async def start_driver(self):
        self.driver_starts += 1
        return types.SimpleNamespace(stop=self.stop_driver)

    async def stop_driver(self):
        self.driver_stops += 1

    async def new_browser(self, driver, **kwargs):
        browser, calls = FakeBrowser(), []
        context = FakeContext(FakePage(calls), calls)
        async def new_context():
            calls.append(("new_context",))
            return context
        browser.new_context = new_context
        browser.fixture_context, browser.fixture_calls = context, calls
        self.browsers.append(browser)
        self.launches.append(kwargs)
        return browser

    async def initialize(self, account, profile, browser):
        context = browser.fixture_context
        value = self.ns["Account"](None, context, context.page,
                                   self.host.lifecycles[account])
        value._observed_scope = self.host._observed_scope(account)
        value.browser = browser
        self.host.accounts[account] = value
        return value

    async def handle_account(self, request):
        self.seen.append(dict(request))
        return {"fixture": True}

    def send(self, command, **extra):
        self.sequence += 1
        self.owner.accept(dict(id=self.sequence, incarnation=self.incarnation,
                               cmd=command, **extra))
        return self.sequence

    async def frame(self, predicate):
        async def wait():
            while True:
                for frame in self.writer.frames:
                    if predicate(frame):
                        return frame
                self.writer.changed.clear()
                await self.writer.changed.wait()
        return await asyncio.wait_for(wait(), 2)

    async def reply(self, request_id):
        return await self.frame(lambda frame: frame.get("id") == request_id)

    async def opened(self, account=None):
        identity = dict(account=account or str(uuid.uuid4()), scope=str(uuid.uuid4()),
                        application_generation=7)
        self.assertTrue((await self.reply(self.send("scope_open", **identity)))["ok"])
        return self.owner.scopes[identity["scope"]]

    async def command(self, scope, command="mint", **extra):
        return await self.reply(self.send(command, **scope.identity(), **extra))

    async def seal(self, scope):
        finish = str(uuid.uuid4())
        self.send("scope_seal", **scope.identity(), finish=finish)
        return await self.frame(lambda f: f.get("kind") == "scope_cleanup"
                                and f.get("finish") == finish)

    async def forget(self, scope):
        result = await self.reply(self.send("scope_forget", **scope.identity(),
                                           finish=scope.finish))
        self.assertTrue(result["ok"])

    async def asyncTearDown(self):
        for gate in self.gates:
            gate.set()
        for browser in self.browsers:
            browser.release.set()
            browser.fixture_context.page_release.set()
        self.writer.release.set()
        try:
            await asyncio.wait_for(self.owner.shutdown(), 3)
        finally:
            if self.owner.profile_fd is not None:
                os.close(self.owner.profile_fd)
                self.owner.profile_fd = None

    async def test_allocation_denial_without_browser_has_confirmed_empty_cleanup(self):
        scope = await self.opened()
        life = scope.lifecycle
        life.cooling["device"] = 600.0
        self.ns["memory_snapshot"] = lambda: {"allocation_headroom_bytes": 0}
        result = await self.command(scope)
        self.assertFalse(result["ok"])
        self.assertIn("90% memory watermark", result["error"])
        self.assertEqual((self.browsers, self.driver_starts), ([], 0))
        self.assertFalse(scope.launch_started)
        self.assertEqual(life.phase, "absent")
        self.assertEqual(scope.lifecycle_incarnation, life.incarnation)
        self.assertEqual(life.active_commands, 0)
        self.assertEqual((await self.seal(scope))["outcome"], "closed")
        self.assertEqual(life.cooling, {"device": 600.0})
        await self.forget(scope)
        self.ns["memory_snapshot"] = lambda: {"allocation_headroom_bytes": 1048576}
        reopened = await self.opened(scope.account)
        self.assertTrue((await self.command(reopened))["ok"])
        self.assertIs(reopened.lifecycle, life)
        self.assertEqual(len(self.browsers), 1)

    async def test_clean_close_and_reopen_preserve_ledger_profile_and_route(self):
        scope = await self.opened()
        proxy = "http://route.invalid:8000"
        self.assertTrue((await self.command(scope, proxy=proxy))["ok"])
        acc, life = self.host.accounts[scope.account], scope.lifecycle
        acc.dev_cooling["device"] = time.monotonic() + 600
        acc.dev_reqs["device"] = deque([time.monotonic()])
        acc.reserve_day, acc.reserve_used, acc.last_mint = "2030-01-01", 2, 42.0
        profile = self.root / scope.account / "fixture-identity"
        profile.write_text("same-device")
        revision, incarnation = scope.proxy_revision, life.incarnation
        self.assertEqual((await self.seal(scope))["outcome"], "closed")
        self.assertEqual(life.phase, "absent")
        self.assertIsNone(life.browser)
        self.assertNotIn(scope.account, self.host.accounts)
        self.assertEqual(self.host.proxies[scope.account], proxy)
        self.assertEqual(self.host.proxy_revisions[scope.account], revision)
        await self.forget(scope)
        new_scope = await self.opened(scope.account)
        self.assertTrue((await self.command(new_scope, proxy=proxy))["ok"])
        reopened = self.host.accounts[scope.account]
        self.assertIsNot(reopened, acc)
        self.assertIs(reopened.lifecycle, life)
        self.assertIs(reopened.dev_cooling, acc.dev_cooling)
        self.assertIs(reopened.dev_reqs, acc.dev_reqs)
        self.assertGreater(reopened.dev_cooling["device"], time.monotonic())
        self.assertEqual(len(reopened.dev_reqs["device"]), 1)
        self.assertEqual((reopened.reserve_day, reopened.reserve_used, reopened.last_mint),
                         ("2030-01-01", 2, 42.0))
        self.assertEqual(profile.read_text(), "same-device")
        self.assertEqual(life.incarnation, incarnation + 1)
        self.assertEqual(new_scope.proxy_revision, revision)

    async def test_new_scope_changes_or_clears_route_only_after_confirmed_close(self):
        account = str(uuid.uuid4())
        for revision, proxy in enumerate(("http://a.invalid:8000",
                                         "http://b.invalid:8001", None), 1):
            with self.subTest(proxy=proxy):
                scope = await self.opened(account)
                self.assertTrue((await self.command(scope, proxy=proxy))["ok"])
                self.assertEqual(scope.proxy_revision, revision)
                self.assertEqual(self.host.proxy_revisions[account], revision)
                self.assertEqual(self.host.proxies.get(account), proxy)
                expected = {} if proxy is None else {"proxy": {"server": proxy}}
                actual = {k: v for k, v in self.launches[-1].items() if k == "proxy"}
                self.assertEqual(actual, expected)
                self.assertEqual((await self.seal(scope))["outcome"], "closed")
                await self.forget(scope)

    async def test_mid_scope_proxy_change_is_rejected_without_mutation(self):
        scope = await self.opened()
        proxy = "http://route.invalid:8000"
        self.assertTrue((await self.command(scope, proxy=proxy))["ok"])
        result = await self.command(scope, proxy="http://other.invalid:8001")
        self.assertFalse(result["ok"])
        self.assertIn("scope proxy changed", result["error"])
        self.assertEqual(self.host.proxies[scope.account], proxy)
        self.assertEqual(self.host.proxy_revisions[scope.account], scope.proxy_revision)
        self.assertEqual((len(self.browsers), len(self.seen)), (1, 1))
        self.assertEqual((await self.seal(scope))["outcome"], "closed")

    async def test_scope_rejects_replaced_route_revision_before_account_work(self):
        scope = await self.opened()
        self.assertTrue((await self.command(scope))["ok"])
        self.host.proxy_revisions[scope.account] += 1
        result = await self.command(scope)
        self.assertFalse(result["ok"])
        self.assertIn("stale account proxy revision", result["error"])
        self.assertEqual(len(self.seen), 1)
        self.assertEqual(len(self.browsers), 1)

    async def test_contextual_initializer_uses_exact_lifecycle_and_owned_rpcs(self):
        scope = await self.opened()
        self.host._initialize_account = self.real_initialize
        result = await self.command(scope, "prepare_inference_authorization",
            project=PROJECT, path="/ai/v3/responses/", cookie_header="session=" + LOGIN)
        self.assertTrue(result["ok"])
        self.assertEqual(result["result"]["source"], "framer_wrapper_direct")
        acc, browser = self.host.accounts[scope.account], self.browsers[0]
        self.assertIs(acc.lifecycle, scope.lifecycle)
        self.assertIs(acc.dev_cooling, scope.lifecycle.cooling)
        self.assertIs(acc.dev_reqs, scope.lifecycle.requests)
        self.assertIs(acc._observed_scope, scope)
        self.assertEqual([call[0] for call in browser.fixture_calls],
                         ["new_context", "new_page", "add_cookies", "goto", "evaluate", "cookies"])
        self.assertEqual(self.owner.background_count, 0)
        self.assertEqual((await self.seal(scope))["outcome"], "closed")

    async def test_initialization_failure_has_only_the_observed_closer(self):
        scope = await self.opened()
        competing = []
        async def fail_initialize(*args):
            raise RuntimeError("fixture initialization failed")
        def forbidden_close(*args):
            competing.append(args)
            raise AssertionError("default closer used in observed mode")
        self.host._initialize_account = fail_initialize
        self.host._start_close = forbidden_close
        self.assertFalse((await self.command(scope))["ok"])
        self.assertEqual((await self.seal(scope))["outcome"], "unconfirmed")
        self.assertEqual(competing, [])
        self.assertEqual(self.browsers[0].calls, 1)
        self.assertEqual(scope.failure, "account_initialization_failed")
        self.assertIsNone(scope.lifecycle.close_task)
        self.assertIs(scope.lifecycle.browser, self.browsers[0])
        self.assertEqual(scope.lifecycle.phase, "unconfirmed")

    async def test_unscoped_account_and_launch_reject_before_local_mutation(self):
        account = str(uuid.uuid4())
        for operation in (self.host._account, self.host._launch):
            with self.assertRaisesRegex(RuntimeError, "admission closed"):
                await operation(account, "http://unowned.invalid:8000")
        self.assertNotIn(account, self.host.lifecycles)
        self.assertNotIn(account, self.host.proxies)
        self.assertFalse((self.root / account).exists())
        self.assertEqual((self.driver_starts, self.browsers), (0, []))

    async def test_browser_loss_cannot_settle_or_relaunch_the_account(self):
        scope = await self.opened()
        self.assertTrue((await self.command(scope))["ok"])
        scope.lifecycle.cooling["device"] = 600.0
        self.browsers[0].disconnect()
        self.assertEqual((await self.seal(scope))["outcome"], "unconfirmed")
        self.assertEqual(self.browsers[0].calls, 0)
        self.assertEqual(self.host._browser_status(scope.account)["state"], "unconfirmed")
        self.assertEqual(scope.lifecycle.cooling, {"device": 600.0})
        with self.assertRaises(RuntimeError):
            await self.host._account(scope.account)
        self.assertEqual(len(self.browsers), 1)

    async def test_missing_close_ack_retains_owned_browser_and_ledger(self):
        scope = await self.opened()
        self.assertTrue((await self.command(scope))["ok"])
        scope.lifecycle.requests["device"] = deque([123.0])
        self.browsers[0].failure = True
        self.assertEqual((await self.seal(scope))["outcome"], "unconfirmed")
        self.assertIs(self.host.lifecycles[scope.account], scope.lifecycle)
        self.assertIs(scope.lifecycle.browser, self.browsers[0])
        self.assertEqual(list(scope.lifecycle.requests["device"]), [123.0])
        self.assertEqual(self.browsers[0].calls, 1)
        self.assertFalse(scope.browsers[0].closed)

    async def test_cleanup_rejects_replaced_lifecycle_incarnation_or_browser(self):
        for changed in ("lifecycle", "incarnation", "browser"):
            with self.subTest(changed=changed):
                scope = await self.opened()
                self.assertTrue((await self.command(scope))["ok"])
                life = scope.lifecycle
                if changed == "lifecycle":
                    self.host.lifecycles[scope.account] = self.ns["AccountLifecycle"]()
                elif changed == "incarnation":
                    life.incarnation += 1
                else:
                    life.browser = object()
                self.assertEqual((await self.seal(scope))["outcome"], "unconfirmed")
                self.assertIn(scope.account, self.host.accounts)

    async def test_default_pool_commands_cannot_bypass_observed_scope(self):
        scope = await self.opened()
        self.assertTrue((await self.command(scope))["ok"])
        for command in ("register_proxy", "browser_status", "close_idle", "close", "rotate", "eval"):
            with self.subTest(command=command), self.assertRaises(RuntimeError):
                await self.host.handle(dict(cmd=command, **scope.identity()))
        self.assertEqual(len(self.browsers), 1)
        self.assertEqual(self.browsers[0].calls, 0)
        self.assertEqual(scope.lifecycle.phase, "resident")

    async def test_default_close_cannot_compete_with_scope_cleanup(self):
        scope = await self.opened()
        self.assertTrue((await self.command(scope))["ok"])
        with self.assertRaisesRegex(RuntimeError, "scopes own"):
            await self.host._close_account(scope.account)
        with self.assertRaisesRegex(RuntimeError, "scopes own"):
            await self.host.close()
        self.assertFalse(self.host._closed)
        self.assertEqual(self.browsers[0].calls, 0)
        self.assertEqual((await self.seal(scope))["outcome"], "closed")
        self.assertEqual(self.browsers[0].calls, 1)

    async def test_scope_open_rejects_an_unconfirmed_lifecycle(self):
        account = str(uuid.uuid4())
        life = self.ns["AccountLifecycle"]()
        life.phase, life.cooling["device"] = "unconfirmed", 600.0
        self.host.lifecycles[account] = life
        result = await self.reply(self.send("scope_open", account=account,
            scope=str(uuid.uuid4()), application_generation=7))
        self.assertFalse(result["ok"])
        self.assertEqual(self.owner.scopes, {})
        self.assertEqual(self.browsers, [])
        self.assertIs(self.host.lifecycles[account], life)
        self.assertEqual(life.cooling, {"device": 600.0})

    async def test_seal_joins_active_command_before_lifecycle_settlement(self):
        scope = await self.opened()
        entered, release = asyncio.Event(), asyncio.Event()
        self.gates.append(release)
        async def held(request):
            entered.set()
            await release.wait()
            return True
        self.host._handle = held
        request_id = self.send("mint", **scope.identity())
        await asyncio.wait_for(entered.wait(), 2)
        finish = str(uuid.uuid4())
        ack = await self.reply(self.send("scope_seal", **scope.identity(), finish=finish))
        self.assertTrue(ack["ok"])
        self.assertEqual(scope.lifecycle.active_commands, 1)
        self.assertEqual(scope.lifecycle.phase, "resident")
        self.assertEqual(self.browsers[0].calls, 0)
        release.set()
        self.assertTrue((await self.reply(request_id))["ok"])
        result = await self.frame(lambda f: f.get("kind") == "scope_cleanup"
                                  and f.get("finish") == finish)
        self.assertEqual(result["outcome"], "closed")
        self.assertEqual(scope.lifecycle.active_commands, 0)
        self.assertEqual(scope.lifecycle.phase, "absent")

    async def test_scope_cleanup_preserves_another_accounts_residency_and_route(self):
        first, second = await self.opened(), await self.opened()
        self.assertTrue((await self.command(first, proxy="http://a.invalid:8000"))["ok"])
        self.assertTrue((await self.command(second, proxy="http://b.invalid:8001"))["ok"])
        other = self.host.accounts[second.account]
        other.dev_cooling["device"] = 600.0
        self.assertEqual((await self.seal(first))["outcome"], "closed")
        self.assertIs(self.host.accounts[second.account], other)
        self.assertTrue(other.browser.is_connected())
        self.assertEqual(other.browser.calls, 0)
        self.assertEqual(other.dev_cooling, {"device": 600.0})
        self.assertEqual(self.host.proxies[second.account], "http://b.invalid:8001")
        self.assertEqual(self.host.proxy_revisions[second.account], second.proxy_revision)

    async def test_contextual_initializer_rejects_lifecycle_replaced_during_rpc(self):
        scope, created = await self.opened(), asyncio.Event()
        self.host._initialize_account = self.real_initialize
        async def paused_browser(driver, **kwargs):
            browser = await self.new_browser(driver, **kwargs)
            browser.fixture_context.page_release.clear()
            self.gates.append(browser.fixture_context.page_release)
            created.set()
            return browser
        self.ns["AsyncNewBrowser"] = paused_browser
        request_id = self.send("prepare_inference_authorization", **scope.identity(),
            project=PROJECT, path="/ai/v3/responses/", cookie_header="session=" + LOGIN)
        await asyncio.wait_for(created.wait(), 2)
        context = self.browsers[0].fixture_context
        await asyncio.wait_for(context.page_started.wait(), 2)
        replacement = self.ns["AccountLifecycle"]()
        self.host.lifecycles[scope.account] = replacement
        context.page_release.set()
        self.assertFalse((await self.reply(request_id))["ok"])
        self.assertNotIn(scope.account, self.host.accounts)
        self.assertIs(self.host.lifecycles[scope.account], replacement)
        self.assertEqual((await self.seal(scope))["outcome"], "unconfirmed")
        self.assertEqual(self.browsers[0].calls, 1)


    async def test_driver_start_failure_is_not_empty_cleanup_success(self):
        scope = await self.opened()
        async def failed_start():
            raise RuntimeError("fixture driver startup failed")
        self.ns["async_playwright"] = lambda: types.SimpleNamespace(start=failed_start)
        self.assertFalse((await self.command(scope))["ok"])
        self.assertTrue(scope.launch_started)
        self.assertEqual(self.browsers, [])
        self.assertEqual((await self.seal(scope))["outcome"], "unconfirmed")
        self.assertEqual(scope.lifecycle.phase, "unconfirmed")
        self.assertEqual(scope.failure, "driver_start_unconfirmed")


if __name__ == "__main__":
    unittest.main()
