"""Exercise the packaged account lifecycle with fake browser ports."""
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

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_host import app
from framerize_host.browser import driver


class FakeBrowser:
    def __init__(self):
        self.connected, self.failure, self.disconnect_on_ack = True, False, True
        self.listeners, self.contexts = [], []
        self.called, self.acknowledged, self.release = asyncio.Event(), asyncio.Event(), asyncio.Event()
        self.release.set()
        self.calls = 0
        self._impl_obj = types.SimpleNamespace(_should_close_connection_on_close=False,
            _channel=types.SimpleNamespace(send=self.send))

    def on(self, event, callback):
        assert event == "disconnected"
        self.listeners.append(callback)

    def remove_listener(self, event, callback):
        assert event == "disconnected"
        self.listeners.remove(callback)

    def is_connected(self):
        return self.connected

    def disconnect(self):
        self.connected = False
        for callback in tuple(self.listeners):
            callback()

    async def send(self, method, calculator, params):
        assert method == "close" and calculator is None and set(params) == {"reason"}
        self.calls += 1
        self.called.set()
        await self.release.wait()
        if self.failure:
            raise RuntimeError("fixture close acknowledgement lost")
        if self.disconnect_on_ack:
            self.disconnect()
        self.acknowledged.set()

    async def close(self):
        raise AssertionError("lifecycle must not use the error-suppressing close API")


class ResidentLifecycleTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="resident-lifecycle-")
        self.tasks, self.browsers = [], []
        self.ns = dict(asyncio=asyncio, os=os, uuid=uuid, time=time, Path=Path,
            asynccontextmanager=asynccontextmanager, ROOT=self.directory.name,
            prepared_memory_browser=lambda: None, prepared_memory_addon=lambda: None,
            trace=lambda *args, **kwargs: None, emit=lambda *args, **kwargs: None,
            verify_close_protocol=lambda: None, BROWSER_STARTUP_RESERVE=1024,
            memory_snapshot=lambda: {"allocation_headroom_bytes": 1048576},
            MemoryPressureError=RuntimeError)
        self.ns["browser_runtime"] = lambda: (self.ns["AsyncNewBrowser"], None)
        self.ns.update(Host=app.Host, Account=app.Account, BrowserCloseReceipt=driver.BrowserCloseReceipt)
        overrides = {name: self.ns[name] for name in (
            'ROOT', 'prepared_memory_browser', 'prepared_memory_addon', 'trace',
            'emit', 'BROWSER_STARTUP_RESERVE', 'MemoryPressureError', 'browser_runtime')}
        overrides['memory_snapshot'] = lambda: self.ns['memory_snapshot']()
        settings = patch.multiple(app, **overrides)
        settings.start()
        self.addCleanup(settings.stop)
        protocol = patch.object(driver, 'verify_close_protocol',
                                lambda: self.ns['verify_close_protocol']())
        protocol.start()
        self.addCleanup(protocol.stop)
        self.host = self.ns["Host"]()
        self.real_launch = self.host._launch
        self.host._launch = self.fake_launch

    async def fake_launch(self, account, proxy=None):
        life = self.host.lifecycles[account]
        browser = FakeBrowser()
        self.browsers.append(browser)
        life.browser = browser
        life.close_receipt = self.ns["BrowserCloseReceipt"](browser)
        acc = self.ns["Account"](None, object(), object(), life)
        acc.browser = browser
        self.host.accounts[account] = acc
        return acc

    async def opened(self, account="account-a"):
        async with self.host._use_account(account) as acc:
            return acc

    def request(self, account="account-a"):
        status = self.host._browser_status(account)
        return dict(cmd="close_idle", account=account, host_generation=status["host_generation"],
                    account_incarnation=status["account_incarnation"])

    @asynccontextmanager
    async def pending_rpc(self, account="account-a"):
        entered, release = asyncio.Event(), asyncio.Event()
        async def raw():
            entered.set()
            await release.wait()
        waiter = asyncio.create_task(self.host._rpc(account, raw))
        self.tasks.append(waiter)
        try:
            await asyncio.wait_for(entered.wait(), 1)
            yield waiter
        finally:
            release.set()
            await asyncio.gather(waiter, return_exceptions=True)
            await asyncio.gather(*self.host.lifecycles[account].activity.pending,
                                 return_exceptions=True)

    def registration(self, account="account-a", proxy=None):
        previous = self.host.proxy_revisions.get(account, 0)
        return dict(cmd="register_proxy", account=account, expected_revision=previous,
                    revision=previous + 1, proxy=proxy)

    async def asyncTearDown(self):
        for task in self.tasks:
            task.cancel()
        await asyncio.gather(*self.tasks, return_exceptions=True)
        for browser in self.browsers:
            browser.release.set()
            browser.disconnect()
        closing = [life.close_task for life in self.host.lifecycles.values() if life.close_task]
        if closing:
            await asyncio.wait_for(asyncio.gather(*closing, return_exceptions=True), 1)
        self.host._playwright = None
        # These fixtures deliberately disconnect browsers without an ack.
        # Drain their tasks without treating teardown as a clean-close result.
        await asyncio.wait_for(asyncio.gather(self.host.close(), return_exceptions=True), 1)
        self.directory.cleanup()

    async def test_status_never_launches_an_absent_account(self):
        status = await self.host.handle(dict(cmd="browser_status", account="missing"))
        self.assertEqual(status["state"], "absent")
        self.assertEqual(status["account_incarnation"], 0)
        self.assertEqual(self.host.lifecycles, {})
        self.assertEqual(self.browsers, [])

    async def test_closed_account_retains_limits_and_profile_on_reopen(self):
        acc = await self.opened()
        profile = Path(self.directory.name) / "account-a"
        profile.mkdir()
        identity = profile / "framerize-devices.json"
        identity.write_text('["fixture-device"]')
        acc.dev_cooling["fixture-device"] = time.monotonic() + 600
        acc.dev_reqs["fixture-device"] = deque([time.monotonic()])
        acc.reserve_day, acc.reserve_used, acc.last_mint = "2030-01-01", 3, 123.0
        before = self.request()
        closed = await self.host.handle(before)
        self.assertEqual(closed["outcome"], "closed")
        reopened = await self.opened()
        self.assertIsNot(reopened, acc)
        self.assertIs(reopened.dev_cooling, acc.dev_cooling)
        self.assertIs(reopened.dev_reqs, acc.dev_reqs)
        self.assertGreater(reopened.dev_cooling["fixture-device"], time.monotonic())
        self.assertEqual(len(reopened.dev_reqs["fixture-device"]), 1)
        self.assertEqual((reopened.reserve_day, reopened.reserve_used, reopened.last_mint),
                         ("2030-01-01", 3, 123.0))
        self.assertEqual(identity.read_text(), '["fixture-device"]')
        self.assertEqual(self.request()["account_incarnation"], before["account_incarnation"] + 1)

    async def test_active_command_denies_close_without_cancelling_work(self):
        async with self.host._use_account("account-a") as acc:
            closed = await self.host.handle(self.request())
            self.assertEqual(closed["outcome"], "busy")
            self.assertEqual(closed["status"]["active_commands"], 1)
            self.assertTrue(acc.browser.is_connected())
            self.assertEqual(acc.browser.calls, 0)
            self.assertFalse(acc.closing)

    async def test_retained_rpc_is_busy_after_waiter_cancellation(self):
        acc = await self.opened()
        request = self.request()
        async with self.pending_rpc() as waiter:
            waiter.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await waiter
            result = await self.host.handle(request)
            self.assertEqual(result["outcome"], "busy")
            self.assertEqual(result["status"]["state"], "resident")
            self.assertGreater(result["status"]["active_commands"], 0)
            self.assertEqual(acc.browser.calls, 0)
            self.assertIsNone(acc.lifecycle.close_task)
            self.assertFalse(acc.closing)
        self.assertEqual((await self.host.handle(self.request()))["outcome"], "closed")

    async def test_status_counts_retained_rpc_without_blocking_another_account(self):
        acc, other = await self.opened(), await self.opened("account-b")
        async with self.pending_rpc():
            status = await self.host.handle(dict(cmd="browser_status", account="account-a"))
            self.assertEqual(status["state"], "resident")
            self.assertGreater(status["active_commands"], 0)
            closed = await self.host.handle(self.request("account-b"))
            self.assertEqual(closed["outcome"], "closed")
            self.assertFalse(other.browser.is_connected())
            self.assertTrue(acc.browser.is_connected())
            self.assertEqual(acc.browser.calls, 0)
        self.assertEqual(self.host._browser_status("account-a")["active_commands"], 0)

    async def test_disconnected_browser_with_retained_rpc_is_not_confirmed_busy(self):
        acc = await self.opened()
        request = self.request()
        async with self.pending_rpc():
            acc.browser.disconnect()
            result = await self.host.handle(request)
            self.assertEqual(result["outcome"], "unconfirmed")
            self.assertEqual(result["status"]["state"], "unconfirmed")
            self.assertIs(self.host.accounts["account-a"], acc)
            self.assertIsNone(acc.lifecycle.close_task)
            self.assertEqual(acc.browser.calls, 0)

    async def test_closing_one_account_does_not_touch_another(self):
        first, other = await self.opened(), await self.opened("account-b")
        other.dev_cooling["other-device"] = 12345
        await self.host.handle(self.request())
        self.assertFalse(first.browser.is_connected())
        self.assertTrue(other.browser.is_connected())
        self.assertEqual(other.browser.calls, 0)
        self.assertEqual(other.dev_cooling, {"other-device": 12345})
        self.assertIsNot(first.dev_cooling, other.dev_cooling)

    async def test_waiting_launch_is_counted_as_busy(self):
        started, release = asyncio.Event(), asyncio.Event()
        async def launch(account, proxy=None):
            started.set()
            await release.wait()
            return await self.fake_launch(account, proxy)
        async def command():
            async with self.host._use_account("account-a"):
                pass
        self.host._launch = launch
        task = asyncio.create_task(command())
        self.tasks.append(task)
        await asyncio.wait_for(started.wait(), 1)
        result = await self.host.handle(self.request())
        self.assertEqual(result["outcome"], "busy")
        self.assertEqual(result["status"]["state"], "starting")
        self.assertEqual(result["status"]["active_commands"], 1)
        release.set()
        await asyncio.wait_for(task, 1)

    async def test_old_incarnation_and_host_do_not_close_replacement(self):
        await self.opened()
        old = self.request()
        await self.host.handle(old)
        replacement = await self.opened()
        result = await self.host.handle(old)
        self.assertEqual(result["outcome"], "stale_generation")
        wrong_host = dict(self.request(), host_generation="previous-host")
        self.assertEqual((await self.host.handle(wrong_host))["outcome"], "stale_generation")
        self.assertTrue(replacement.browser.is_connected())
        self.assertEqual(replacement.browser.calls, 0)

    async def test_cancelled_close_caller_leaves_cleanup_owned(self):
        acc = await self.opened()
        acc.browser.release.clear()
        caller = asyncio.create_task(self.host.handle(self.request()))
        self.tasks.append(caller)
        await asyncio.wait_for(acc.browser.called.wait(), 1)
        life = self.host.lifecycles["account-a"]
        owned = life.close_task
        caller.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await caller
        self.assertFalse(owned.done())
        with self.assertRaises(RuntimeError):
            await self.host._account("account-a")
        acc.browser.release.set()
        self.assertTrue(await asyncio.wait_for(owned, 1))
        self.assertEqual(self.host._browser_status("account-a")["state"], "absent")
        self.assertEqual(acc.browser.calls, 1)

    async def test_idle_close_joins_started_close_with_retained_rpc(self):
        acc = await self.opened()
        request = self.request()
        acc.browser.release.clear()
        first = asyncio.create_task(self.host.handle(request))
        self.tasks.append(first)
        await asyncio.wait_for(acc.browser.called.wait(), 1)
        owned = acc.lifecycle.close_task
        self.assertTrue(acc.lifecycle.activity.pending)
        entered = asyncio.Event()
        async def again():
            entered.set()
            return await self.host.handle(request)
        second = asyncio.create_task(again())
        self.tasks.append(second)
        await asyncio.wait_for(entered.wait(), 1)
        self.assertFalse(second.done())
        self.assertIs(acc.lifecycle.close_task, owned)
        self.assertEqual(acc.browser.calls, 1)
        acc.browser.release.set()
        results = await asyncio.wait_for(asyncio.gather(first, second), 1)
        self.assertEqual([r["outcome"] for r in results], ["closed", "closed"])
        self.assertTrue(all(r["status"]["state"] == "absent" for r in results))
        self.assertEqual(acc.browser.calls, 1)

    async def test_missing_close_ack_retains_browser_and_limits(self):
        acc = await self.opened()
        acc.dev_cooling["device"] = time.monotonic() + 600
        acc.browser.failure = True
        result = await self.host.handle(self.request())
        self.assertEqual(result["outcome"], "unconfirmed")
        self.assertEqual(result["status"]["state"], "unconfirmed")
        self.assertIs(self.host.accounts["account-a"], acc)
        self.assertIn("device", acc.dev_cooling)
        with self.assertRaises(RuntimeError):
            await self.host._account("account-a")

    async def test_disconnect_without_ack_is_not_absence(self):
        acc = await self.opened()
        acc.browser.disconnect()
        result = await self.host.handle(self.request())
        self.assertEqual(result["outcome"], "unconfirmed")
        self.assertEqual(result["status"]["state"], "unconfirmed")
        self.assertEqual(acc.browser.calls, 0)
        self.assertIs(self.host.accounts["account-a"], acc)

    async def test_ack_without_disconnect_keeps_capacity(self):
        acc = await self.opened()
        acc.browser.disconnect_on_ack = False
        caller = asyncio.create_task(self.host.handle(self.request()))
        self.tasks.append(caller)
        await asyncio.wait_for(acc.browser.acknowledged.wait(), 1)
        life = self.host.lifecycles["account-a"]
        self.assertEqual(life.phase, "closing")
        self.assertFalse(caller.done())
        with self.assertRaises(RuntimeError):
            await self.host._account("account-a")
        acc.browser.disconnect()
        result = await asyncio.wait_for(caller, 1)
        self.assertEqual(result["outcome"], "closed")
        self.assertEqual(acc.browser.listeners, [])

    async def test_failed_launch_handshake_is_not_retried_as_absent(self):
        async def failed_launch(*args, **kwargs):
            raise RuntimeError("fixture handshake failed")
        self.host._playwright = object()
        self.host._launch = self.real_launch
        self.ns["AsyncNewBrowser"] = failed_launch
        with self.assertRaisesRegex(RuntimeError, "handshake"):
            await self.opened()
        status = self.host._browser_status("account-a")
        self.assertEqual(status["state"], "unconfirmed")
        self.assertEqual(status["active_commands"], 0)
        with self.assertRaisesRegex(RuntimeError, "cleanup"):
            await self.opened()
        self.assertEqual(self.host._browser_status("account-a")["account_incarnation"], 1)

    async def test_memory_admission_failure_without_launch_stays_absent(self):
        async def forbidden_launch(*args, **kwargs):
            raise AssertionError("memory denial must occur before launch")
        self.host._playwright = object()
        self.host._launch = self.real_launch
        self.ns["AsyncNewBrowser"] = forbidden_launch
        self.ns["memory_snapshot"] = lambda: {"allocation_headroom_bytes": 0}
        with self.assertRaisesRegex(RuntimeError, "insufficient memory headroom"):
            await self.opened()
        status = self.host._browser_status("account-a")
        self.assertEqual(status["state"], "absent")
        self.assertEqual(status["active_commands"], 0)
        self.assertEqual(self.browsers, [])

    async def test_pressure_refusal_preserves_running_account_and_warm_reuse(self):
        acc = await self.opened()
        self.ns["memory_snapshot"] = lambda: {"allocation_headroom_bytes": -1}
        async with self.host._use_account("account-a"):
            with self.assertRaisesRegex(RuntimeError, "insufficient memory headroom"):
                async with self.host._allocation(1024):
                    self.fail("a new allocation must be refused")
            self.assertTrue(acc.browser.connected)
            self.assertEqual(acc.active_requests, 1)
            self.assertEqual(self.host._browser_status("account-a")["state"], "resident")
            self.assertIs(await self.opened(), acc)
        self.assertEqual(acc.active_requests, 0)
        self.assertEqual(self.host._reserved_bytes, 0)
        self.assertEqual(acc.browser.calls, 0)

    async def test_failed_initialization_retains_unconfirmed_browser(self):
        browser = FakeBrowser()
        browser.failure = True
        self.browsers.append(browser)
        async def launch(*args, **kwargs):
            return browser
        async def initialize(*args, **kwargs):
            raise RuntimeError("fixture initialization failed")
        self.host._playwright = object()
        self.host._launch = self.real_launch
        self.host._initialize_account = initialize
        self.ns["AsyncNewBrowser"] = launch
        with self.assertRaisesRegex(RuntimeError, "initialization"):
            await self.opened()
        life = self.host.lifecycles["account-a"]
        self.assertEqual(life.phase, "unconfirmed")
        self.assertIs(life.browser, browser)
        self.assertEqual(life.active_commands, 0)

    async def test_unsupported_close_protocol_does_not_release_capacity(self):
        acc = await self.opened()
        def unsupported():
            raise RuntimeError("fixture unsupported protocol")
        self.ns["verify_close_protocol"] = unsupported
        result = await self.host.handle(self.request())
        self.assertEqual(result["outcome"], "unconfirmed")
        self.assertEqual(acc.browser.calls, 0)
        self.assertTrue(acc.browser.is_connected())

    async def test_reopened_account_clears_its_proxy_without_touching_another(self):
        async def inspect(req):
            return dict(self.host.proxies)
        self.host._handle = inspect
        await self.host.handle(dict(cmd="eval", account="account-a", proxy="http://a.invalid:1234"))
        await self.host.handle(dict(cmd="eval", account="account-b", proxy="http://b.invalid:4321"))
        await self.host.handle(self.request())
        changed = await self.host.handle(self.registration())
        self.assertEqual(changed["outcome"], "registered")
        result = await self.host.handle(dict(cmd="eval", account="account-a", proxy=None,
                                            proxy_revision=changed["route"]["revision"]))
        self.assertNotIn("account-a", result)
        self.assertEqual(result["account-b"], "http://b.invalid:4321")
        self.assertEqual(len(self.browsers), 3)

    async def test_omitted_proxy_retains_the_configured_account_route(self):
        async def inspect(req):
            return dict(self.host.proxies)
        self.host._handle = inspect
        await self.host.handle(dict(cmd="eval", account="account-a", proxy="http://a.invalid:1234"))
        await self.host.handle(self.request())
        result = await self.host.handle(dict(cmd="eval", account="account-a"))
        self.assertEqual(result["account-a"], "http://a.invalid:1234")
        self.assertEqual(len(self.browsers), 2)

    async def test_live_account_cannot_silently_change_proxy(self):
        async def inspect(req):
            return dict(self.host.proxies)
        self.host._handle = inspect
        await self.host.handle(dict(cmd="eval", account="account-a", proxy="http://a.invalid:1234"))
        with self.assertRaisesRegex(RuntimeError, "requires confirmed browser closure"):
            await self.host.handle(dict(cmd="eval", account="account-a", proxy=None))
        self.assertEqual(self.host.proxies["account-a"], "http://a.invalid:1234")
        self.assertEqual(len(self.browsers), 1)
        self.assertEqual(self.browsers[0].calls, 0)

    async def test_proxy_registration_is_nonlaunching_and_idempotent(self):
        request = self.registration(proxy="http://a.invalid:1234")
        first = await self.host.handle(request)
        again = await self.host.handle(request)
        self.assertEqual(first, again)
        self.assertEqual(first["outcome"], "registered")
        self.assertEqual(first["status"]["state"], "absent")
        self.assertEqual(first["route"], {"revision": 1, "proxy": "http://a.invalid:1234"})
        self.assertEqual(self.host.lifecycles, {})
        self.assertEqual(self.browsers, [])
        self.assertIsNone(self.host._playwright)

    async def test_proxy_registration_refuses_live_and_unconfirmed_browsers(self):
        await self.opened()
        for phase in ("resident", "unconfirmed"):
            self.host.lifecycles["account-a"].phase = phase
            result = await self.host.handle(self.registration(proxy="http://new.invalid:4321"))
            self.assertEqual(result["outcome"], "busy")
            self.assertNotIn("account-a", self.host.proxies)
            self.assertNotIn("account-a", self.host.proxy_revisions)
            self.assertEqual(self.browsers[0].calls, 0)

    async def test_old_proxy_revision_cannot_launch_or_close_a_new_browser(self):
        async def inspect(req):
            return True
        self.host._handle = inspect
        registered = await self.host.handle(self.registration(proxy="http://a.invalid:1234"))
        self.assertEqual(registered["outcome"], "registered")
        for cmd in ("eval", "close", "rotate"):
            with self.assertRaisesRegex(RuntimeError, "stale account proxy revision"):
                await self.host.handle(dict(cmd=cmd, account="account-a", proxy_revision=0))
        self.assertEqual(self.browsers, [])
        await self.host.handle(dict(cmd="eval", account="account-a", proxy_revision=1,
                                    proxy="http://a.invalid:1234"))
        with self.assertRaisesRegex(RuntimeError, "stale account proxy revision"):
            await self.host.handle(dict(cmd="close", account="account-a", proxy_revision=0))
        self.assertEqual(self.browsers[0].calls, 0)
        self.assertTrue(self.browsers[0].is_connected())

    async def test_same_revision_cannot_change_an_absent_accounts_route(self):
        await self.host.handle(self.registration(proxy="http://a.invalid:1234"))
        with self.assertRaisesRegex(RuntimeError, "requires confirmed browser closure and registration"):
            await self.host.handle(dict(cmd="eval", account="account-a", proxy_revision=1,
                                        proxy="http://other.invalid:4321"))
        self.assertEqual(self.host.proxies["account-a"], "http://a.invalid:1234")
        self.assertEqual(self.browsers, [])

    async def test_registration_during_launch_is_busy_but_another_account_is_independent(self):
        entered, release = asyncio.Event(), asyncio.Event()
        original = self.host._launch
        async def held_launch(account, proxy=None):
            entered.set()
            await release.wait()
            return await original(account, proxy)
        self.host._launch = held_launch
        opening = asyncio.create_task(self.opened())
        self.tasks.append(opening)
        await asyncio.wait_for(entered.wait(), 1)
        refused = await self.host.handle(self.registration(proxy="http://a.invalid:1234"))
        self.assertEqual(refused["outcome"], "busy")
        self.assertEqual(refused["status"]["active_commands"], 1)
        accepted = await self.host.handle(self.registration("account-b", "http://b.invalid:4321"))
        self.assertEqual(accepted["outcome"], "registered")
        self.assertEqual(self.browsers, [])
        release.set()
        await asyncio.wait_for(opening, 1)
        self.assertNotIn("account-a", self.host.proxies)
        self.assertEqual(self.host.proxies["account-b"], "http://b.invalid:4321")

    async def test_invalid_proxy_registration_revisions_never_mutate_state(self):
        for expected, revision in ((0, 0), (0, True), (-1, 0), (0, 2), (2**64 - 1, 2**64)):
            with self.assertRaises(ValueError):
                await self.host.handle(dict(cmd="register_proxy", account="account-a",
                    expected_revision=expected, revision=revision, proxy="http://a.invalid:1234"))
        self.assertEqual(self.host.proxies, {})
        self.assertEqual(self.host.proxy_revisions, {})
        self.assertEqual(self.host.lifecycles, {})
        self.assertEqual(self.browsers, [])

    async def test_startup_registration_accepts_rusts_unstarted_route_revision(self):
        result = await self.host.handle(dict(cmd="register_proxy", account="account-a",
            expected_revision=4, revision=5, proxy="http://saved.invalid:1234"))
        self.assertEqual(result["outcome"], "registered")
        self.assertEqual(result["route"]["revision"], 5)
        self.assertEqual(self.browsers, [])
        self.assertEqual(self.host.lifecycles, {})

    async def test_stale_registration_cannot_overwrite_a_newer_saved_route(self):
        first = self.registration(proxy="http://first.invalid:1234")
        await self.host.handle(first)
        await self.host.handle(self.registration(proxy="http://second.invalid:4321"))
        refused = await self.host.handle(first)
        self.assertEqual(refused["outcome"], "stale")
        self.assertEqual(refused["route"], {"revision": 2, "proxy": "http://second.invalid:4321"})
        self.assertEqual(self.browsers, [])

    async def test_invalid_proxy_values_never_change_registration_or_start_browser(self):
        for value in ("", 1, False, []):
            with self.assertRaises(ValueError):
                await self.host.handle(self.registration(proxy=value))
        self.assertEqual(self.host.proxies, {})
        self.assertEqual(self.host.proxy_revisions, {})
        self.assertEqual(self.browsers, [])

    async def test_inconsistent_absent_state_is_not_accepted(self):
        acc = await self.opened()
        self.host.lifecycles["account-a"].phase = "absent"
        self.assertEqual(self.host._browser_status("account-a")["state"], "unconfirmed")
        self.host.lifecycles.pop("account-a")
        result = await self.host.handle(self.request())
        self.assertEqual(result["outcome"], "unconfirmed")
        self.assertTrue(acc.browser.is_connected())


if __name__ == "__main__":
    unittest.main(verbosity=2)
