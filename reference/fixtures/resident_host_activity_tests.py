"""Inert host activity fixtures. No host import, SDK, browser or network.

Extract only source definitions; all browser ports and heartbeat file reads
are synthetic. Execution belongs to the runtime owner. Added SOURCE-ONLY/UNRUN.
"""
import ast
import asyncio
import inspect
import json
import os
from pathlib import Path
import random
import sys
import types
import unittest
import uuid
from contextlib import asynccontextmanager

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
from resident_activity import ResidentRefused


class Port:
    def __init__(self, result=None, held=False):
        self.result, self.error, self.calls, self.task = result, None, [], None
        self.entered, self.release = asyncio.Event(), asyncio.Event()
        if not held:
            self.release.set()

    async def __call__(self, *args, **kwargs):
        self.calls.append((args, kwargs))
        self.task = asyncio.current_task()
        self.entered.set()
        await self.release.wait()
        if self.error:
            raise self.error
        return self.result


class FakePage(types.SimpleNamespace):
    __hash__ = object.__hash__
    __eq__ = object.__eq__


class ReceiptBrowser:
    def __init__(self):
        self.connected, self.listeners, self.close_calls = True, [], []
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

    async def send(self, method, calculator, params):
        assert method == "close" and calculator is None and set(params) == {"reason"}
        self.close_calls.append(params)
        self.connected = False
        for callback in tuple(self.listeners):
            callback()

    async def close(self):
        raise AssertionError("cleanup must use the acknowledged close protocol")


class SleepGate:
    def __init__(self):
        self.calls, self.arrivals = [], asyncio.Queue()

    async def __call__(self, delay):
        release = asyncio.Event()
        self.calls.append((delay, release))
        self.arrivals.put_nowait(release)
        await release.wait()

    async def next(self):
        return await asyncio.wait_for(self.arrivals.get(), 1)


class HostActivityTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        source = Path(__file__).resolve().parents[2] / "scripts/camoufox_host.py"
        tree = ast.parse(source.read_text())
        names = {"Device", "BrowserCloseReceipt", "AccountLifecycle", "Account", "Host",
                 "human_burst"}
        selected = [n for n in tree.body if isinstance(n, (ast.ClassDef,
            ast.AsyncFunctionDef)) and n.name in names]
        self.assertEqual({n.name for n in selected}, names)
        self.tasks, self.ports, self.effects, self.clock = [], [], [], 1000.0
        self.asyncio = types.SimpleNamespace(**vars(asyncio))
        self.ns = dict(asyncio=self.asyncio, os=os, uuid=uuid, random=random,
            json=json, Path=Path, asynccontextmanager=asynccontextmanager,
            time=types.SimpleNamespace(monotonic=lambda: self.clock),
            ROOT="unused-fixture-root", BROWSER_STARTUP_RESERVE=1024,
            verify_close_protocol=lambda: None,
            HEARTBEAT_S=60, AI_START_JS="start", AI_POLL_JS="poll",
            AI_CANCEL_JS="cancel", SAFE_CLICK_JS="click", WARM_JS="warm",
            trace=lambda *a, **k: None, emit=lambda *a, **k: None)
        exec(compile(ast.Module(body=selected, type_ignores=[]), str(source), "exec"), self.ns)
        self.host = self.ns["Host"].__new__(self.ns["Host"])
        self.host.accounts, self.host.lifecycles = {}, {}
        self.host.proxies, self.host.proxy_revisions = {}, {}
        self.host.generation, self.host._observed = "fixture-host", None
        self.host._closed, self.host._playwright = False, None
        self.host._driver_start_task = None
        self.host.lock, self.host._allocation_lock = asyncio.Lock(), asyncio.Lock()
        self.host._reserved_bytes = 0
        self.host.idle_gc_interval = self.host.idle_gc_pressure_interval = 1
        self.acc = self.make_account("a")
        self.other = self.make_account("b")

    def port(self, result=None, held=False):
        port = Port(result, held)
        self.ports.append(port)
        return port

    def make_account(self, name):
        life = self.host.lifecycles.setdefault(name, self.ns["AccountLifecycle"]())
        browser = types.SimpleNamespace(is_connected=lambda: True, contexts=[])
        context = types.SimpleNamespace(browser=browser, pages=[])
        page = FakePage(context=context, is_closed=lambda: False,
            evaluate=self.port(False), request_gc=self.port(),
            wait_for_timeout=self.port(), on=lambda *a: None,
            remove_listener=lambda *a: None, mouse=types.SimpleNamespace(
                move=self.port(), wheel=self.port(), click=self.port()))
        context.pages.append(page)
        browser.contexts.append(context)
        acc = self.ns["Account"](None, context, page, life)
        acc.browser, acc.last_used, acc.last_gc = browser, 0, 0
        life.browser, life.phase = browser, "resident"
        self.host.accounts[name] = acc
        return acc

    def prepare_launch(self, *, held=False):
        life = self.ns["AccountLifecycle"]()
        self.host.lifecycles["launch"] = life
        browser = ReceiptBrowser()
        launch = self.port(browser, held)
        self.host._browser_runtime = (launch, None)
        self.host._playwright = object()
        self.host._memory_browser = self.host._memory_addon = None
        # No environment, private profile, resource or browser package reads.
        self.ns["os"] = types.SimpleNamespace(environ={}, path=os.path,
            makedirs=lambda *a, **k: None)
        self.ns["memory_snapshot"] = lambda: {"allocation_headroom_bytes": 2048}

        async def initialize(account, profile, created):
            self.assertIs(created, browser)
            self.assertIs(life.close_receipt.browser, browser)
            self.effects.append(("initialize", life.close_receipt))
            acc = self.ns["Account"](None, object(), object(), life)
            acc.browser = created
            acc._observed_scope = self.host._observed_scope(account)
            self.host.accounts[account] = acc
            return acc

        self.host._initialize_account = initialize
        return life, browser, launch

    def observe_launch(self):
        source = Path(__file__).resolve().parents[2] / "scripts/observed_sidecar.py"
        names = {"Scope", "OwnedBrowser"}
        selected = [n for n in ast.parse(source.read_text()).body
                    if isinstance(n, ast.ClassDef) and n.name in names]
        self.assertEqual({n.name for n in selected}, names)
        exec(compile(ast.Module(body=selected, type_ignores=[]), str(source), "exec"), self.ns)
        owner = types.SimpleNamespace(accounts={}, closed=False)
        scope = self.ns["Scope"](owner, "launch", 1, "fixture-scope")
        owner.accounts["launch"] = scope
        self.host._observed = owner
        self.host._observed_scope_open(scope)
        return scope

    def spawn(self, coro):
        task = asyncio.create_task(coro)
        self.tasks.append(task)
        return task

    def refused(self, life=None):
        with self.assertRaises(ResidentRefused):
            (life or self.acc.lifecycle).activity.acquire(str(uuid.uuid4()))

    async def available(self, life=None):
        lease = (life or self.acc.lifecycle).activity.acquire(str(uuid.uuid4()))
        self.assertEqual((await lease.settle("not_started", True)).state, "released")

    async def cancel(self, task):
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task

    async def asyncTearDown(self):
        for task in self.tasks:
            task.cancel()
        for port in self.ports:
            port.release.set()
        await asyncio.gather(*self.tasks, return_exceptions=True)
        pending = {task for life in self.host.lifecycles.values()
                   for task in life.activity.pending}
        pending.update(life.close_task for life in self.host.lifecycles.values()
                       if life.close_task is not None)
        if self.host._driver_start_task is not None:
            pending.add(self.host._driver_start_task)
        if pending:
            await asyncio.wait_for(asyncio.gather(*pending, return_exceptions=True), 1)

    async def test_every_command_is_refused_before_route_or_browser_side_effects(self):
        lease = self.acc.lifecycle.activity.acquire(str(uuid.uuid4()))
        commands = ("begin", "complete", "mint", "ai_post", "set_cookie", "eval",
                    "register_proxy", "browser_status", "close_idle", "close",
                    "rotate", "prepare_inference_authorization", "browser_baseline_once")
        for command in commands:
            with self.subTest(command=command), self.assertRaises(ResidentRefused):
                await self.host.handle(dict(cmd=command, account="a",
                    proxy="http://synthetic.invalid", expected_revision=0, revision=1))
        self.assertEqual(self.host.proxies, {})
        self.assertEqual(self.host.proxy_revisions, {})
        self.assertFalse(self.acc.closing)
        self.assertFalse(any(port.calls for port in self.ports))
        self.assertEqual(self.acc.lifecycle.active_commands, 0)
        self.assertEqual((await lease.settle("not_started", True)).state, "released")

    async def test_absent_status_and_registration_keep_no_lifecycle_behavior(self):
        status = await self.host.handle(dict(cmd="browser_status", account="absent"))
        self.assertEqual(status["state"], "absent")
        result = await self.host.handle(dict(cmd="register_proxy", account="absent",
            expected_revision=0, revision=1, proxy="http://synthetic.invalid"))
        self.assertEqual(result["outcome"], "registered")
        self.assertNotIn("absent", self.host.lifecycles)

    async def test_a_reserved_account_does_not_block_another_account(self):
        self.acc.lifecycle.activity.acquire(str(uuid.uuid4()))
        status = await self.host.handle(dict(cmd="browser_status", account="b"))
        self.assertEqual(status["state"], "resident")
        await self.available(self.other.lifecycle)

    async def test_raw_rpc_outlives_cancelled_waiter_and_retains_uncertainty(self):
        port = self.port("late result", held=True)
        waiter = self.spawn(self.host._rpc("a", port))
        await asyncio.wait_for(port.entered.wait(), 1)
        await self.cancel(waiter)
        self.assertIn(port.task, self.acc.lifecycle.activity.pending)
        self.assertFalse(port.task.cancelled())

        self.refused()
        await self.available(self.other.lifecycle)
        port.release.set()
        await asyncio.wait_for(asyncio.gather(*self.acc.lifecycle.activity.pending), 1)
        self.assertFalse(self.acc.lifecycle.activity.pending)
        self.refused()
        async with self.host._activity("a"):
            pass  # The conservative hold only bars future resident admission.

    def job_ports(self, *, held=False):
        ports = {"start": self.port(), "poll": self.port({"status": 200}, held),
                 "cancel": self.port(False)}
        async def evaluate(kind, *args):
            return await ports[kind](*args)
        self.acc.page.evaluate = evaluate
        return ports

    def run_job(self):
        return self.host._run_job(self.acc.page, "/synthetic", {}, "token",
                                  "bearer", "job", "device", 30)

    async def test_completed_job_releases_activity_after_cleanup_rpc(self):
        ports = self.job_ports()
        self.assertEqual(await self.run_job(), {"status": 200})
        self.assertEqual(len(ports["cancel"].calls), 1)
        await self.available()

    async def test_cancelled_job_keeps_poll_rpc_and_does_not_treat_abort_as_settlement(self):
        ports = self.job_ports(held=True)
        task = self.spawn(self.run_job())
        await asyncio.wait_for(ports["poll"].entered.wait(), 1)
        await self.cancel(task)
        self.assertEqual(len(ports["cancel"].calls), 1)
        self.assertIn(ports["poll"].task, self.acc.lifecycle.activity.pending)
        ports["poll"].release.set()
        await asyncio.wait_for(asyncio.gather(*self.acc.lifecycle.activity.pending), 1)
        self.refused()
        self.assertFalse(self.acc.closing)

    async def test_timeout_wraps_waiter_and_never_discards_the_actual_rpc(self):
        port = self.port(held=True)
        with self.assertRaises(asyncio.TimeoutError):
            await asyncio.wait_for(self.host._rpc("a", port), 0.01)
        self.assertTrue(port.entered.is_set())
        self.assertIn(port.task, self.acc.lifecycle.activity.pending)
        self.assertFalse(port.task.cancelled())
        self.refused()

    async def test_rpc_failure_keeps_a_resident_hold_even_when_caught(self):
        port = self.port()
        port.error = RuntimeError("synthetic protocol uncertainty")
        with self.assertRaises(RuntimeError):
            await self.host._rpc("a", port)
        self.refused()
        self.assertIsNotNone(self.acc.lifecycle.resident_uncertain)

    async def test_completed_raw_rpc_releases_its_activity(self):
        self.assertEqual(await self.host._rpc("a", self.port("done")), "done")
        await self.available()

    async def test_cancelled_command_stops_followup_work_but_keeps_raw_owner(self):
        port = self.port(held=True)
        async def command(req):
            await self.host._rpc("a", port)
            self.effects.append("must not run")
        self.host._handle = command
        task = self.spawn(self.host.handle(dict(cmd="set_cookie", account="a")))
        await asyncio.wait_for(port.entered.wait(), 1)
        await self.cancel(task)
        self.assertEqual(self.acc.lifecycle.active_commands, 0)
        self.assertEqual(self.acc.active_requests, 0)
        self.assertIn(port.task, self.acc.lifecycle.activity.pending)
        port.release.set()
        await asyncio.wait_for(asyncio.gather(*self.acc.lifecycle.activity.pending), 1)
        self.assertEqual(self.effects, [])
        self.refused()

    async def test_creation_task_reserves_before_first_poll_and_until_completion(self):
        port = self.port(held=True)
        async def warm():
            await self.host._rpc("a", port)
        task = self.host._track_task(self.acc, warm())
        self.tasks.append(task)
        self.refused()
        await asyncio.wait_for(port.entered.wait(), 1)
        self.refused()
        port.release.set()
        await asyncio.wait_for(task, 1)
        await self.available()

    async def test_refused_creation_task_never_starts_and_closes_its_coroutine(self):
        self.acc.lifecycle.activity.acquire(str(uuid.uuid4()))
        coro = self.port()()
        with self.assertRaises(ResidentRefused):
            self.host._track_task(self.acc, coro)
        self.assertEqual(inspect.getcoroutinestate(coro), inspect.CORO_CLOSED)

    async def test_human_sleep_is_unreserved_burst_is_gated_and_rpc_survives_cancel(self):
        gate = self.asyncio.sleep = SleepGate()
        move = self.acc.page.mouse.move
        move.release.clear()
        task = self.spawn(self.host._human_loop("a", self.acc.page))
        first = await gate.next()
        await self.available()
        lease = self.acc.lifecycle.activity.acquire(str(uuid.uuid4()))
        first.set()
        second = await gate.next()
        self.assertEqual(move.calls, [])
        await lease.settle("not_started", True)
        second.set()
        await asyncio.wait_for(move.entered.wait(), 1)
        self.refused()
        await self.cancel(task)
        self.assertIn(move.task, self.acc.lifecycle.activity.pending)
        self.assertFalse(move.task.cancelled())

    async def test_cancelled_sleeping_human_loop_leaves_no_activity(self):
        gate = self.asyncio.sleep = SleepGate()
        task = self.host._track_task(self.acc,
            self.host._human_loop("a", self.acc.page), idle_loop=True)
        self.tasks.append(task)
        await gate.next()
        await self.available()
        await self.cancel(task)
        await self.available()

    async def test_heartbeat_waits_do_not_reserve_but_creation_does(self):
        gate = self.asyncio.sleep = SleepGate()
        # The host's operator-file port is synthetic, never the private file.
        self.ns["Path"] = lambda *a: types.SimpleNamespace(expanduser=lambda:
            types.SimpleNamespace(read_text=lambda: "[]"))
        self.acc.devices, self.acc.devpool_ready, self.acc.project = ["d"], True, "p"
        preparation = self.port(types.SimpleNamespace(page=self.acc.page), held=True)
        async def device(*args):
            return await self.host._rpc("a", preparation)
        self.host._dev_page = device
        task = self.spawn(self.host._heartbeat_loop(self.acc, "a"))
        boot = await gate.next()
        await self.available()
        lease = self.acc.lifecycle.activity.acquire(str(uuid.uuid4()))
        boot.set()
        interval = await gate.next()
        interval.set()
        backoff = await gate.next()
        self.assertEqual(preparation.calls, [])
        await lease.settle("not_started", True)
        await self.available()
        backoff.set()
        interval = await gate.next()
        interval.set()
        await asyncio.wait_for(preparation.entered.wait(), 1)
        self.refused()
        await self.cancel(task)
        self.assertIn(preparation.task, self.acc.lifecycle.activity.pending)

    async def test_gc_skips_reserved_account_before_counters_or_browser_calls(self):
        self.acc.lifecycle.activity.acquire(str(uuid.uuid4()))
        before = self.acc.last_gc, self.acc.gc_page
        await self.host._reclaim_idle_memory({"allocation_headroom_bytes": 0})
        self.assertEqual((self.acc.last_gc, self.acc.gc_page), before)
        self.assertEqual(self.acc.page.evaluate.calls, [])
        self.assertEqual(self.acc.page.request_gc.calls, [])
        self.assertEqual(len(self.other.page.request_gc.calls), 1)

    async def test_cancelled_gc_keeps_raw_probe_owned(self):
        port = self.acc.page.evaluate
        port.release.clear()
        task = self.spawn(self.host._reclaim_idle_memory({"allocation_headroom_bytes": 0}))
        await asyncio.wait_for(port.entered.wait(), 1)
        self.refused()
        await self.cancel(task)
        self.assertIn(port.task, self.acc.lifecycle.activity.pending)
        self.assertFalse(port.task.cancelled())
        self.assertEqual(self.acc.page.request_gc.calls, [])

    async def test_gc_collection_itself_stays_owned_after_cancellation(self):
        port = self.acc.page.request_gc
        port.release.clear()
        task = self.spawn(self.host._reclaim_idle_memory({"allocation_headroom_bytes": 0}))
        await asyncio.wait_for(port.entered.wait(), 1)
        await self.cancel(task)
        self.assertIn(port.task, self.acc.lifecycle.activity.pending)
        self.refused()

    async def test_close_retains_its_existing_independent_owner(self):
        close = self.port(held=True)
        life = self.acc.lifecycle
        life.close_receipt = types.SimpleNamespace(browser=life.browser, close=close)
        waiter = self.spawn(self.host._close_account("a"))
        await asyncio.wait_for(close.entered.wait(), 1)
        owner = life.close_task
        await self.cancel(waiter)
        self.assertFalse(owner.cancelled())
        self.assertIn(close.task, life.activity.pending)
        self.refused()
        close.release.set()
        self.assertTrue(await asyncio.wait_for(owner, 1))
        self.assertEqual(life.phase, "absent")
        await self.available(life)

    async def test_monitor_close_and_host_shutdown_refuse_a_held_lease(self):
        self.acc.lifecycle.activity.acquire(str(uuid.uuid4()))
        with self.assertRaises(ResidentRefused):
            await self.host._close_account("a", idle_before=self.clock)
        with self.assertRaises(ResidentRefused):
            await self.host.close()
        self.assertFalse(self.host._closed)
        self.assertFalse(self.acc.closing)
        self.assertIsNone(self.acc.lifecycle.close_task)
        self.assertFalse(self.other.closing)

    async def test_close_does_not_interrupt_pending_ordinary_rpc(self):
        port = self.port(held=True)
        self.spawn(self.host._rpc("a", port))
        await asyncio.wait_for(port.entered.wait(), 1)
        self.assertFalse(await self.host._close_account("a"))
        self.assertFalse(self.acc.closing)
        self.assertFalse(port.task.cancelled())

    async def test_job_deadline_retains_fence_after_successful_abort_rpc(self):
        ports = self.job_ports()
        async def evaluate(kind, *args):
            if kind == "poll":
                self.clock += 31
                return None
            return await ports[kind](*args)
        self.acc.page.evaluate = evaluate
        result = await self.run_job()
        self.assertEqual(result["status"], -1)
        self.assertEqual(len(ports["cancel"].calls), 1)
        self.assertFalse(self.acc.lifecycle.activity.pending)
        self.refused()

    async def test_unstructured_eval_always_retains_resident_uncertainty(self):
        self.acc.page.evaluate.result = "synthetic result"
        result = await self.host.handle(dict(cmd="eval", account="a", expr="fixture"))
        self.assertEqual(result, "synthetic result")
        self.refused()
        self.assertFalse(self.acc.closing)

    async def test_observed_rpc_stays_in_original_calling_task(self):
        self.host._observed = object()
        self.acc.lifecycle.activity.fenced = True
        port = self.port("observed result")
        result = await self.host._rpc("a", port)
        self.assertEqual(result, "observed result")
        self.assertIs(port.task, asyncio.current_task())
        self.assertFalse(self.acc.lifecycle.activity.pending)

    async def test_account_creation_has_no_global_lock_across_launch(self):
        self.host.accounts.clear()
        self.host.lifecycles.clear()
        opening = self.port(held=True)
        async def launch(account, proxy=None):
            if account == "a":
                await opening()
            return self.make_account(account)
        self.host._launch = launch
        first = self.spawn(self.host._account("a"))
        await asyncio.wait_for(opening.entered.wait(), 1)
        self.refused(self.host.lifecycles["a"])
        second = await asyncio.wait_for(self.host._account("b"), 1)
        await self.available(second.lifecycle)
        opening.release.set()
        result = await asyncio.wait_for(first, 1)
        await self.available(result.lifecycle)

    async def test_launch_waiter_cancellation_retains_pending_owner(self):
        life, browser, launch = self.prepare_launch(held=True)
        waiter = self.spawn(self.host._account("launch"))
        await asyncio.wait_for(launch.entered.wait(), 1)
        await self.cancel(waiter)
        self.assertIn(launch.task, life.activity.pending)
        self.assertFalse(launch.task.done())
        self.assertEqual(life.phase, "unconfirmed")
        self.assertIsNone(life.browser)
        self.assertIsNone(life.close_receipt)
        self.assertNotIn("launch", self.host.accounts)
        self.assertEqual(self.effects, [])
        self.assertFalse(await self.host._close_account("launch"))
        self.assertEqual(browser.close_calls, [])
        self.refused(life)
        await self.available(self.other.lifecycle)

    async def test_late_launch_success_has_matching_receipt_and_closes(self):
        life, browser, launch = self.prepare_launch(held=True)
        life.cooling["fixture-device"] = 1234
        waiter = self.spawn(self.host._account("launch"))
        await asyncio.wait_for(launch.entered.wait(), 1)
        await self.cancel(waiter)
        launch.release.set()
        self.assertIs(await asyncio.wait_for(asyncio.shield(launch.task), 1), browser)
        receipt = life.close_receipt
        self.assertIsInstance(receipt, self.ns["BrowserCloseReceipt"])
        self.assertIs(receipt.browser, life.browser)
        self.assertIs(life.browser, browser)
        self.assertEqual(life.phase, "unconfirmed")
        self.assertFalse(life.activity.pending)
        self.assertNotIn("launch", self.host.accounts)
        self.assertEqual(self.effects, [])
        self.assertTrue(await self.host._close_account("launch"))
        self.assertTrue(receipt.acknowledged and receipt.closed)
        self.assertFalse(browser.connected)
        self.assertEqual(browser.listeners, [])
        self.assertEqual(len(browser.close_calls), 1)
        self.assertEqual(life.phase, "absent")
        self.assertIsNone(life.browser)
        self.assertIsNone(life.close_receipt)
        self.assertEqual(life.cooling, {"fixture-device": 1234})
        self.assertTrue(await self.host._close_account("launch"))
        self.assertEqual(len(browser.close_calls), 1)
        self.refused(life)  # Closing does not erase the earlier uncertainty hold.
        await self.available(self.other.lifecycle)

    async def test_ordinary_launch_initializes_with_one_matching_close_receipt(self):
        life, browser, launch = self.prepare_launch()
        acc = await self.host._account("launch")
        receipt = life.close_receipt
        self.assertIsInstance(receipt, self.ns["BrowserCloseReceipt"])
        self.assertIs(receipt.browser, browser)
        self.assertEqual(self.effects, [("initialize", receipt)])
        self.assertEqual(len(browser.listeners), 1)
        self.assertEqual(len(launch.calls), 1)
        self.assertIsNot(launch.task, asyncio.current_task())
        self.assertIs(self.host.accounts["launch"], acc)
        self.assertEqual(acc.resident_launch, (browser, life.incarnation, None, False))
        self.assertEqual(life.phase, "resident")
        await self.available(life)
        self.assertTrue(await self.host._close_account("launch"))
        self.assertTrue(receipt.acknowledged and receipt.closed)
        self.assertEqual(len(browser.close_calls), 1)
        self.assertFalse(browser.connected)
        self.assertNotIn("launch", self.host.accounts)
        self.assertEqual(life.phase, "absent")
        await self.available(life)

    async def test_observed_launch_retains_scope_receipt_and_original_owner(self):
        life, browser, launch = self.prepare_launch()
        scope = self.observe_launch()
        acc = await self.host._account("launch")
        receipt = life.close_receipt
        self.assertIsInstance(receipt, self.ns["OwnedBrowser"])
        self.assertEqual(scope.browsers, [receipt])
        self.assertIs(receipt.browser, browser)
        self.assertIs(acc.browser, browser)
        self.assertEqual(self.effects, [("initialize", receipt)])
        self.assertEqual(len(browser.listeners), 1)
        self.assertTrue(scope.launch_started)
        self.assertIs(launch.task, asyncio.current_task())
        self.assertFalse(life.activity.pending)
        await receipt.close()
        self.host._observed_scope_closed(scope)
        self.assertTrue(receipt.closed)
        self.assertEqual(len(browser.close_calls), 1)
        self.assertFalse(browser.connected)
        self.assertEqual(browser.listeners, [])
        self.assertIsNone(scope.failure)
        self.assertEqual(life.phase, "absent")
        self.assertIsNone(life.browser)
        self.assertIsNone(life.close_receipt)
        self.assertNotIn("launch", self.host.accounts)

    async def test_observed_launch_cancellation_stays_with_scope_owner(self):
        life, browser, launch = self.prepare_launch(held=True)
        scope = self.observe_launch()
        waiter = self.spawn(self.host._account("launch"))
        await asyncio.wait_for(launch.entered.wait(), 1)
        self.assertIs(launch.task, waiter)
        await self.cancel(waiter)
        launch.release.set()
        self.assertTrue(launch.task.cancelled())
        self.assertFalse(life.activity.pending)
        self.assertEqual(scope.failure, "browser_launch_unconfirmed")
        self.assertEqual(scope.browsers, [])
        self.assertEqual(life.phase, "unconfirmed")
        self.assertIsNone(life.browser)
        self.assertIsNone(life.close_receipt)
        self.assertNotIn("launch", self.host.accounts)
        self.assertEqual(self.effects, [])
        self.assertEqual(browser.close_calls, [])

    async def test_shared_driver_start_survives_one_cancelled_waiter(self):
        driver = object()
        start = self.port(driver, held=True)
        factory = lambda: types.SimpleNamespace(start=start)
        first = self.spawn(self.host._start_driver(factory))
        await asyncio.wait_for(start.entered.wait(), 1)
        second = self.spawn(self.host._start_driver(factory))
        await self.cancel(first)
        self.assertFalse(start.task.cancelled())
        start.release.set()
        await asyncio.wait_for(second, 1)
        self.assertEqual(len(start.calls), 1)
        self.assertIs(self.host._playwright, driver)


if __name__ == "__main__":
    unittest.main(verbosity=2)
