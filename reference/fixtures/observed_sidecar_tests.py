"""Offline tests: no browser, credentials, or network."""
import asyncio
import os
import sys
import tempfile
import types
import unittest
import uuid
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "scripts"))
import observed_sidecar as obs


class FakeWriter:
    def __init__(self):
        self.frames = []
        self.changed, self.started, self.release = asyncio.Event(), asyncio.Event(), asyncio.Event()
        self.release.set()
        self.lock = asyncio.Lock()
        self.block_id = None

    async def send(self, frame):
        async with self.lock:
            if frame.get("id") == self.block_id and self.block_id is not None:
                self.started.set()
                await self.release.wait()
            self.frames.append(frame)
            self.changed.set()

    async def close(self):
        self.release.set()


class FakeBrowser:
    def __init__(self):
        self.listeners = []
        self.connected = True
        self.called, self.ack = asyncio.Event(), asyncio.Event()
        self.ack.set()
        self.failure = False
        self.auto_disconnect = True
        self._impl_obj = types.SimpleNamespace(_should_close_connection_on_close=False,
            _channel=types.SimpleNamespace(send=self.send))

    def on(self, _, callback):
        self.listeners.append(callback)

    def remove_listener(self, _, callback):
        self.listeners.remove(callback)

    def is_connected(self):
        return self.connected

    def disconnect(self):
        self.connected = False
        for callback in tuple(self.listeners):
            callback()

    async def send(self, method, calculator, params):
        assert method == "close" and calculator is None
        self.called.set()
        await self.ack.wait()
        if self.failure:
            raise RuntimeError("fixture close acknowledgement lost")
        if self.auto_disconnect:
            self.disconnect()


class FakeHost:
    def __init__(self):
        self.accounts, self.proxies = {}, {}
        self._playwright = None
        self._command_tasks = {}
        self.seen = []
        self.result = {"status": 200, "body": "fixture", "usage": None}
        self.started, self.release = asyncio.Event(), asyncio.Event()
        self.release.set()

    def _observed_scope_open(self, scope):
        # This protocol fixture has no resident-pool lifecycle state.
        pass

    def _observed_scope_closed(self, scope):
        self.accounts.pop(scope.account, None)
        self.proxies.pop(scope.account, None)

    async def handle(self, req):
        self.seen.append(req["id"])
        self.started.set()
        await self.release.wait()
        return self.result


class ObserverTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="observer-offline-")
        self.root = Path(self.directory.name)
        self.nonce, self.incarnation = str(uuid.uuid4()), str(uuid.uuid4())
        (self.root / ".observer-owner").write_text(self.nonce)
        self.host, self.writer = FakeHost(), FakeWriter()
        self.owner = obs.ObservedHost(self.host, self.root, self.incarnation, self.nonce)
        self.owner.writer = self.writer
        self.sequence, self.browsers, self.gates = 0, [], []
        self.protocol = patch.object(obs, "verify_close_protocol", return_value=None)
        self.protocol.start()
        request_id = self.send("observer_hello", scopes=4, commands=8)
        await self.frame(lambda f: f.get("id") == request_id)

    def send(self, cmd, **extra):
        self.sequence += 1
        req = dict(id=self.sequence, cmd=cmd, incarnation=self.incarnation)
        req.update(extra)
        self.owner.accept(req)
        return self.sequence

    async def frame(self, predicate):
        async def waiting():
            while True:
                for frame in self.writer.frames:
                    if predicate(frame):
                        return frame
                self.writer.changed.clear()
                await self.writer.changed.wait()
        return await asyncio.wait_for(waiting(), 2)

    async def opened(self, with_browser=True):
        identity = dict(account=str(uuid.uuid4()), scope=str(uuid.uuid4()), application_generation=7)
        request_id = self.send("scope_open", **identity)
        await self.frame(lambda f: f.get("id") == request_id)
        scope = self.owner.scopes[identity["scope"]]
        if with_browser:
            browser = FakeBrowser()
            self.browsers.append(browser)
            scope.browser_started(browser)
        return scope

    async def seal(self, scope, stop=False):
        finish = str(uuid.uuid4())
        self.send("scope_stop" if stop else "scope_seal", **scope.identity(), finish=finish)
        return await self.frame(lambda f: f.get("kind") == "scope_cleanup" and f.get("finish") == finish)

    async def asyncTearDown(self):
        self.host.release.set()
        self.writer.release.set()
        for browser in self.browsers:
            browser.ack.set()
        for gate in self.gates:
            gate.set()
        try:
            await asyncio.wait_for(self.owner.shutdown(), 3)
        finally:
            if self.owner.profile_fd is not None:
                os.close(self.owner.profile_fd)
            self.protocol.stop()
            self.directory.cleanup()

    async def test_closed_receipt_binds_complete_identity(self):
        scope = await self.opened()
        out = await self.seal(scope)
        self.assertEqual(out["outcome"], "closed")
        self.assertEqual(out["incarnation"], self.incarnation)
        self.assertEqual(out["extent"], "account_browser_closed")
        for key, value in scope.identity().items():
            self.assertEqual(out[key], value)

    async def test_result_keeps_body_status_usage_and_queue_wait(self):
        scope = await self.opened()
        self.host.result = {"status": 403, "body": "risk-refused λ\n" * 6000,
                            "usage": "fixture-usage", "queue_wait_ms": 812}
        request_id = self.send("ai_post", **scope.identity())
        out = await self.frame(lambda f: f.get("id") == request_id)
        self.assertIs(out["result"], self.host.result)
        self.assertEqual(out["result"]["queue_wait_ms"], 812)
        self.assertEqual((await self.seal(scope))["outcome"], "closed")

    async def test_replacement_incarnation_cannot_submit_work(self):
        scope = await self.opened()
        request_id = self.send("mint", **scope.identity(), incarnation=str(uuid.uuid4()))
        self.assertFalse((await self.frame(lambda f: f.get("id") == request_id))["ok"])
        self.assertFalse(self.host.seen)
        self.assertEqual((await self.seal(scope))["outcome"], "unconfirmed")

    async def test_wrong_account_and_generation_are_rejected(self):
        scope = await self.opened()
        identity = dict(scope.identity(), account=str(uuid.uuid4()), application_generation=8)
        request_id = self.send("mint", **identity)
        self.assertFalse((await self.frame(lambda f: f.get("id") == request_id))["ok"])
        self.assertFalse(self.host.seen)
        self.assertEqual((await self.seal(scope))["outcome"], "unconfirmed")

    async def test_arbitrary_eval_has_no_observed_success_default(self):
        scope = await self.opened()
        request_id = self.send("eval", **scope.identity(), expr="must not run")
        self.assertFalse((await self.frame(lambda f: f.get("id") == request_id))["ok"])
        self.assertFalse(self.host.seen)

    async def test_lost_browser_is_not_successful_teardown(self):
        scope = await self.opened()
        self.browsers[-1].disconnect()
        self.assertEqual((await self.seal(scope))["outcome"], "unconfirmed")

    async def test_close_acknowledgement_failure_is_unconfirmed(self):
        scope = await self.opened()
        self.browsers[-1].failure = True
        self.assertEqual((await self.seal(scope))["outcome"], "unconfirmed")

    async def test_close_requires_acknowledgement_and_disconnect(self):
        scope = await self.opened()
        browser = self.browsers[-1]
        browser.auto_disconnect = False
        finish = str(uuid.uuid4())
        self.send("scope_seal", **scope.identity(), finish=finish)
        await asyncio.wait_for(browser.called.wait(), 1)
        self.assertIsNone(scope.terminal)
        browser.disconnect()
        out = await self.frame(lambda f: f.get("finish") == finish and f.get("kind") == "scope_cleanup")
        self.assertEqual(out["outcome"], "closed")

    async def test_cleanup_retains_pending_browser_rpc_after_close(self):
        scope = await self.opened()
        gate = asyncio.Event()
        self.gates.append(gate)
        scope.rpc(gate.wait())
        finish = str(uuid.uuid4())
        self.send("scope_seal", **scope.identity(), finish=finish)
        await asyncio.wait_for(self.browsers[-1].called.wait(), 1)
        self.assertIsNone(scope.terminal)
        gate.set()
        out = await self.frame(lambda f: f.get("kind") == "scope_cleanup" and f.get("finish") == finish)
        self.assertEqual(out["outcome"], "closed")

    async def test_stop_joins_command_instead_of_cancelling_supervisor(self):
        scope = await self.opened()
        self.host.release.clear()
        request_id = self.send("mint", **scope.identity())
        await asyncio.wait_for(self.host.started.wait(), 1)
        finish = str(uuid.uuid4())
        self.send("scope_stop", **scope.identity(), finish=finish)
        self.assertTrue(scope.stopped.is_set())
        self.assertEqual(scope.commands[request_id].cancelling(), 0)
        self.assertIsNone(scope.terminal)
        self.host.release.set()
        out = await self.frame(lambda f: f.get("kind") == "scope_cleanup" and f.get("finish") == finish)
        self.assertEqual(out["outcome"], "closed")

    async def test_blocked_reply_keeps_ownership_but_accepts_stop(self):
        scope = await self.opened()
        self.writer.block_id = self.sequence + 1
        self.writer.release.clear()
        request_id = self.send("mint", **scope.identity())
        await asyncio.wait_for(self.writer.started.wait(), 1)
        finish = str(uuid.uuid4())
        self.send("scope_stop", **scope.identity(), finish=finish)
        self.assertTrue(scope.stopped.is_set())
        self.assertIn(request_id, scope.commands)
        self.assertIsNone(scope.terminal)
        self.assertFalse(self.browsers[-1].called.is_set())
        self.writer.release.set()
        await self.frame(lambda f: f.get("kind") == "scope_cleanup" and f.get("finish") == finish)
        self.assertNotIn(request_id, scope.commands)

    async def test_repeated_stop_does_not_interrupt_background_join(self):
        scope = await self.opened()
        started, cancelled, release = asyncio.Event(), asyncio.Event(), asyncio.Event()
        self.gates.append(release)
        async def background():
            started.set()
            try:
                await asyncio.Event().wait()
            except asyncio.CancelledError:
                cancelled.set()
                await release.wait()
        task = scope.background_task(background())
        await asyncio.wait_for(started.wait(), 1)
        finish = str(uuid.uuid4())
        self.send("scope_stop", **scope.identity(), finish=finish)
        await asyncio.wait_for(cancelled.wait(), 1)
        for _ in range(100):
            scope.stop_background()
        self.assertEqual(task.cancelling(), 1)
        self.assertFalse(task.done())
        self.assertIsNone(scope.terminal)
        release.set()
        out = await self.frame(lambda f: f.get("kind") == "scope_cleanup" and f.get("finish") == finish)
        self.assertEqual(out["outcome"], "closed")

    async def test_sealed_scope_does_not_start_new_background_producer(self):
        scope = await self.opened()
        await self.seal(scope)
        touched = []
        async def extra():
            touched.append(True)
        await scope.background_task(extra())
        self.assertFalse(touched)

    async def test_background_budget_is_global_and_rejects_before_start(self):
        scope = await self.opened()
        self.owner.background_count = obs.MAX_RECORDS
        touched = []
        async def extra():
            touched.append(True)
        coro = extra()
        try:
            with self.assertRaises(RuntimeError):
                scope.background_task(coro)
            self.assertIsNone(coro.cr_frame)
            self.assertFalse(touched)
        finally:
            self.owner.background_count = 0

    async def test_capacity_rejection_preserves_admitted_command(self):
        scope = await self.opened()
        self.owner.command_limit = 1
        self.host.release.clear()
        first = self.send("mint", **scope.identity())
        await asyncio.wait_for(self.host.started.wait(), 1)
        second = self.send("mint", **scope.identity())
        self.assertFalse((await self.frame(lambda f: f.get("id") == second))["ok"])
        self.assertIn(first, scope.commands)
        self.assertEqual(len(self.host.seen), 1)

    async def test_background_and_rpc_register_before_eager_first_poll(self):
        scope = await self.opened()
        original_create_task = asyncio.create_task
        for kind in ("background", "rpc"):
            with self.subTest(kind=kind):
                records = scope.background if kind == "background" else scope.rpcs
                launch = scope.background_task if kind == "background" else scope.rpc
                touched, captured = [], []
                async def work():
                    touched.append(True)
                    self.assertIn(captured[0], records)
                    self.assertIs(captured[0].task, asyncio.current_task())
                    return 41
                def eager_first_poll(runner):
                    self.assertEqual(len(records), 1)
                    record = next(iter(records))
                    captured.append(record)
                    self.assertEqual(getattr(self.owner, kind + "_count"), 1)
                    self.assertFalse(record.joined.done())
                    # Drive the exact eager-start boundary without requiring
                    # a particular Python version's eager task factory.
                    self.assertIs(runner.send(None), record.ready)
                    self.assertFalse(touched)
                    return original_create_task(runner)
                with patch.object(obs.asyncio, "create_task", new=eager_first_poll):
                    task = launch(work())
                self.assertEqual(await task, 41)
                await captured[0].joined
                self.assertFalse(records)
                self.assertEqual(getattr(self.owner, kind + "_count"), 0)
        self.assertEqual((await self.seal(scope))["outcome"], "closed")

    async def test_task_factory_failure_releases_unstarted_reservation(self):
        scope = await self.opened()
        for kind in ("background", "rpc"):
            with self.subTest(kind=kind):
                records = scope.background if kind == "background" else scope.rpcs
                launch = scope.background_task if kind == "background" else scope.rpc
                touched, captured = [], []
                async def work():
                    touched.append(True)
                def rejected(runner):
                    self.assertEqual(len(records), 1)
                    captured.append(next(iter(records)))
                    raise RuntimeError("fixture task factory refused launch")
                coro = work()
                with patch.object(obs.asyncio, "create_task", new=rejected):
                    with self.assertRaisesRegex(RuntimeError, "fixture task factory"):
                        launch(coro)
                self.assertFalse(touched)
                self.assertIsNone(coro.cr_frame)
                self.assertTrue(captured[0].joined.done())
                self.assertFalse(records)
                self.assertEqual(getattr(self.owner, kind + "_count"), 0)
        self.assertEqual((await self.seal(scope))["outcome"], "closed")

    async def test_stop_during_background_launch_keeps_and_joins_owner(self):
        scope = await self.opened()
        original_create_task = asyncio.create_task
        touched, captured = [], []
        finish = str(uuid.uuid4())
        async def work():
            touched.append(True)
        def stopped_during_launch(runner):
            record = next(iter(scope.background))
            captured.append(record)
            self.assertIsNone(record.task)
            scope.seal(finish, stop=True)
            self.assertTrue(record.cancel_requested)
            self.assertFalse(record.joined.done())
            return original_create_task(runner)
        coro = work()
        with patch.object(obs.asyncio, "create_task", new=stopped_during_launch):
            task = scope.background_task(coro)
        with self.assertRaises(asyncio.CancelledError):
            await task
        await captured[0].joined
        self.assertFalse(touched)
        self.assertIsNone(coro.cr_frame)
        out = await self.frame(lambda f: f.get("kind") == "scope_cleanup" and f.get("finish") == finish)
        self.assertEqual(out["outcome"], "closed")
        self.assertEqual(self.owner.background_count, 0)

    async def test_rpc_cancellation_before_first_poll_is_not_clean(self):
        scope = await self.opened()
        touched = []
        async def work():
            touched.append(True)
        coro = work()
        task = scope.rpc(coro)
        record = next(iter(scope.rpcs))
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        await record.joined
        self.assertFalse(touched)
        self.assertIsNone(coro.cr_frame)
        self.assertEqual(self.owner.rpc_count, 0)
        self.assertEqual((await self.seal(scope))["outcome"], "unconfirmed")

    async def test_rpc_admission_stays_closed_after_cleanup_snapshot(self):
        scope = await self.opened()
        await self.seal(scope)
        touched = []
        async def work():
            touched.append(True)
        coro = work()
        with self.assertRaisesRegex(RuntimeError, "RPC admission closed"):
            scope.rpc(coro)
        self.assertIsNone(coro.cr_frame)
        self.assertFalse(touched)
        self.assertFalse(scope.rpcs)
        self.assertEqual(self.owner.rpc_count, 0)

    async def test_rpc_budget_is_global_and_rejects_before_start(self):
        scope = await self.opened()
        self.owner.rpc_count = obs.MAX_RECORDS
        touched = []
        async def work():
            touched.append(True)
        coro = work()
        try:
            with self.assertRaisesRegex(RuntimeError, "RPC capacity"):
                scope.rpc(coro)
            self.assertIsNone(coro.cr_frame)
            self.assertFalse(touched)
            self.assertFalse(scope.rpcs)
        finally:
            self.owner.rpc_count = 0

    async def test_profile_lock_prevents_second_process_owner(self):
        with self.assertRaises(OSError):
            obs.ObservedHost(FakeHost(), self.root, str(uuid.uuid4()), self.nonce)

    async def test_existing_profile_data_cannot_be_adopted(self):
        (self.root / "existing-account").mkdir()
        with self.assertRaises(RuntimeError):
            obs.ObservedHost(FakeHost(), self.root, str(uuid.uuid4()), self.nonce)

    async def test_unsupported_close_protocol_prevents_admission(self):
        self.owner.negotiated = False
        with patch.object(obs, "verify_close_protocol", side_effect=RuntimeError("unsupported fixture")):
            request_id = self.send("observer_hello", scopes=4, commands=8)
        self.assertFalse((await self.frame(lambda f: f.get("id") == request_id))["ok"])
        self.assertFalse(self.owner.scopes)
        self.assertTrue(self.owner.closed)

    async def test_terminal_record_lives_until_consumed(self):
        scope = await self.opened()
        receipt = await self.seal(scope)
        self.assertIn(scope.scope_id, self.owner.scopes)
        request_id = self.send("scope_forget", **scope.identity(), finish=receipt["finish"])
        reply = await self.frame(lambda f: f.get("id") == request_id)
        self.assertTrue(reply["result"]["forgotten"])
        self.assertNotIn(scope.scope_id, self.owner.scopes)

    async def test_unknown_cancel_does_not_confirm_account_cleanup(self):
        scope = await self.opened()
        request_id = self.send("cancel", request_id=999)
        self.assertFalse((await self.frame(lambda f: f.get("id") == request_id))["ok"])
        self.assertEqual((await self.seal(scope))["outcome"], "unconfirmed")

    async def test_timed_out_poll_is_reawaited_without_losing_result(self):
        scope = await self.opened()
        calls, injected = [], []
        result = {"status": 200, "body": "complete JSON", "usage": "fixture"}
        async def evaluate(kind, args):
            calls.append(kind)
            return result if kind == "poll" else True
        page = types.SimpleNamespace(evaluate=evaluate)
        original = asyncio.wait_for
        async def timeout_once(awaitable, timeout):
            if timeout == 8 and not injected:
                injected.append(True)
                raise asyncio.TimeoutError
            return await original(awaitable, timeout)
        with patch.object(obs.asyncio, "wait_for", new=timeout_once):
            got = await obs.run_job(scope, page, "/fixture", {}, "fixture", None,
                                    "job", None, 40, "start", "poll", "cancel")
        self.assertIs(got, result)
        self.assertEqual(calls.count("poll"), 1)
        self.assertEqual((await self.seal(scope))["outcome"], "closed")

    async def test_late_start_remains_owned_through_browser_close(self):
        scope = await self.opened()
        release, started = asyncio.Event(), asyncio.Event()
        self.gates.append(release)
        async def evaluate(kind, args):
            if kind == "start":
                started.set()
                await release.wait()
            return True
        page = types.SimpleNamespace(evaluate=evaluate)
        original = asyncio.wait_for
        async def timeout_start(awaitable, timeout):
            if timeout == 30:
                await started.wait()
                raise asyncio.TimeoutError
            return await original(awaitable, timeout)
        with patch.object(obs.asyncio, "wait_for", new=timeout_start):
            with self.assertRaises(asyncio.TimeoutError):
                await obs.run_job(scope, page, "/fixture", {}, "fixture", None,
                                  "job", None, 40, "start", "poll", "cancel")
        self.assertEqual(len(scope.rpcs), 1)
        finish = str(uuid.uuid4())
        self.send("scope_seal", **scope.identity(), finish=finish)
        await original(self.browsers[-1].called.wait(), 1)
        self.assertIsNone(scope.terminal)
        release.set()
        await self.frame(lambda f: f.get("kind") == "scope_cleanup" and f.get("finish") == finish)
