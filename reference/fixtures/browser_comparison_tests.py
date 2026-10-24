"""Inert comparison fixtures. UNRUN by the source-only implementation owner."""
import asyncio
import copy
from contextlib import asynccontextmanager
import importlib.util
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "_comparison_existing_fixture", Path(__file__).with_name("contextual_authorization_tests.py"))
OLD = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(OLD)
C = OLD.CONTEXT
DEVICE = "fixture-observed-device"

OBSERVER_SPEC = importlib.util.spec_from_file_location(
    "_comparison_observer_fixture", Path(__file__).with_name("observed_sidecar_tests.py"))
OBSERVER = importlib.util.module_from_spec(OBSERVER_SPEC)
OBSERVER_SPEC.loader.exec_module(OBSERVER)


def envelope(turn):
    return {"projectId": OLD.PROJECT, "sessionId": "fixture-conversation", "turnId": turn,
            "request": {"model": "fixture-model", "stream": False, "input": "fixture prompt"}}


def response():
    return {"id": "fixture-response", "status": "completed", "output": [
        {"type": "message", "content": [{"type": "output_text", "text": "fixture answer"}]}]}


def wire(value=None, **changes):
    text = json.dumps(response() if value is None else value)
    result = {"status": 200, "body": text, "bytes": len(text.encode()),
              "content_type": "application/json", "complete": True,
              "started": 1, "finished": 1, "settled": True, "failure": None,
              "elapsed_ms": 1200}
    result.update(changes)
    return result


class ReadinessHandle:
    def __init__(self, calls):
        self.calls = calls

    async def dispose(self):
        self.calls.append(("dispose_readiness",))


class Harness(OLD.Harness):
    def __init__(self):
        super().__init__()
        self.scope.contextual_authorization = False
        self.scope.commands, self.scope.launch_started = {}, False
        self.scope.comparison_phase, self.scope.comparison = None, None
        self.scope.comparison_next = "failed"
        self.scope.generation, self.scope.scope_id = 7, "fixture-scope"
        self.scope.identity = lambda: {"account": self.account_id,
            "application_generation": self.scope.generation, "scope": self.scope.scope_id}
        self.scope.fail = lambda reason: setattr(self.scope, "failure", reason)
        self.account.browser = self.browser
        self.page.device, self.page.main_frame = DEVICE, object()
        self.page.events = {}
        self.page.on = lambda event, callback: self.page.events.setdefault(event, []).append(callback)
        self.page.evaluate = self.evaluate
        self.page.wait_for_function = self.wait_for_function
        self.context.rows = [{"name": "session", "value": OLD.LOGIN},
                             {"name": "__cuid", "value": DEVICE}]
        self.body, self.native_body = envelope("baseline-turn"), envelope("native-turn")
        self.fetch_result, self.fetch_error = wire(), False
        self.fetch_started, self.fetch_release = asyncio.Event(), asyncio.Event()
        self.fetch_release.set()
        self.mints, self.fetches, self.context_reply = 0, 0, None

    async def wait_for_function(self, script, *, timeout, polling):
        # A successful browser reply; this fixture does not evaluate JavaScript.
        self.calls.append(("wait_for_function", script, timeout, polling))
        return ReadinessHandle(self.calls)

    async def evaluate(self, script, args=None):
        if script == C.COMPARISON_SNAPSHOT:
            return {"url": self.page.url, "device": self.page.device}
        if script == C.COMPARISON_MINT:
            self.mints += 1
            self.page.sdk_started.set()
            await self.page.sdk_release.wait()
            if self.page.fail_sdk:
                raise RuntimeError("private-fixture-mint-error")
            return {"before": self.page.url, "after": self.page.url,
                    "token": "private-fixture-token-" + str(self.mints)}
        if script == C.COMPARISON_FETCH:
            self.fetches += 1
            self.fetch_args = args
            self.fetch_started.set()
            await self.fetch_release.wait()
            if self.fetch_error:
                raise RuntimeError("private-fixture-fetch-error")
            return copy.deepcopy(self.fetch_result)
        raise AssertionError("unexpected page operation")

    async def command(self, cmd, **fields):
        request = dict(self.scope.identity(), cmd=cmd, **fields)
        C.reserve_comparison(self.scope, request)
        self.scope.commands[1] = True
        try:
            return await C.comparison_command(self.host, self.account_id, request)
        finally:
            self.scope.commands.clear()
            self.scope.comparison_phase = self.scope.comparison_next
            self.scope.comparison_next = "failed"

    async def prepare_context(self, **changes):
        fields = dict(purpose="baseline_context", project=OLD.PROJECT,
            path="/ai/v3/responses/", cookie_header="session=" + OLD.LOGIN,
            body=self.body, native_body=self.native_body,
            wrapper_url=C.COMPARISON_WRAPPER, wrapper_export=C.COMPARISON_EXPORT)
        fields.update(changes)
        self.context_reply = await self.command("prepare_inference_authorization", **fields)
        return self.context_reply

    def bound_fields(self, body):
        return dict(context_handle=self.context_reply["context_handle"],
                    project=OLD.PROJECT, path=self.context_reply["path"], body=body)

    async def baseline(self, **changes):
        fields = dict(self.bound_fields(self.body), bearer="private-fixture-bearer",
                      cookie_header=self.context_reply["cookie_header"])
        fields.update(changes)
        return await self.command("browser_baseline_once", **fields)

    async def native(self, **changes):
        fields = dict(self.bound_fields(self.native_body), purpose="native_token")
        fields.update(changes)
        return await self.command("prepare_inference_authorization", **fields)


class BrowserComparisonTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.values, self.tasks = [], []

    def fixture(self):
        value = Harness()
        self.values.append(value)
        return value

    async def asyncTearDown(self):
        for value in self.values:
            value.fetch_release.set()
            value.page.sdk_release.set()
        for task in self.tasks:
            if not task.done():
                task.cancel()
        await asyncio.gather(*self.tasks, return_exceptions=True)
        for value in self.values:
            await asyncio.wait_for(value.scope.drain(), 2)

    async def test_distinct_turn_sequence_mints_twice_and_fetches_only_baseline(self):
        value = self.fixture()
        context = await value.prepare_context()
        self.assertNotIn("token", context)
        self.assertEqual((value.mints, value.fetches), (0, 0))
        baseline = await value.baseline()
        self.assertTrue(baseline["accepted"])
        self.assertTrue(baseline["complete_json"] and baseline["nonempty_text"])
        for private in ("fixture answer", "fixture prompt", "private-fixture", OLD.LOGIN, DEVICE):
            self.assertNotIn(private, json.dumps(baseline))
        native = await value.native()
        self.assertEqual((value.mints, value.fetches), (2, 1))
        self.assertEqual(native["context_handle"], context["context_handle"])
        self.assertEqual(native["source"], "framer_wrapper_direct")
        self.assertNotEqual(native["token"], value.fetch_args["token"])
        self.assertEqual(json.loads(value.fetch_args["body"]), value.body)
        self.assertEqual(value.scope.comparison_phase, "complete")
        names = [call[0] for call in value.calls]
        self.assertEqual(names.count("goto"), 1)
        self.assertEqual(names.count("add_cookies"), 1)
        self.assertEqual(value.calls[0][1][0]["name"], "session")

    async def test_turn_ids_must_be_distinct_bounded_owner_strings(self):
        for turn in ("baseline-turn", "", None, 4, "x" * 257, "a\nb", "λ" * 129):
            with self.subTest(turn=turn):
                value = self.fixture()
                value.native_body["turnId"] = turn
                with self.assertRaisesRegex(RuntimeError, "^browser comparison failed$"):
                    await value.prepare_context()
                self.assertEqual(value.calls, [])
                self.assertEqual((value.mints, value.fetches), (0, 0))

    async def test_every_other_envelope_value_is_bound_including_json_types(self):
        changes = ({"sessionId": "another-session"}, {"projectId": "another-project"},
                   {"request": {"model": "another-model", "stream": False}},
                   {"extra": 1}, {"request": {"model": "fixture-model", "stream": False,
                       "input": "fixture prompt", "flag": 1}})
        for change in changes:
            with self.subTest(change=change):
                value = self.fixture()
                value.body["request"]["flag"] = True
                value.native_body = copy.deepcopy(value.body)
                value.native_body["turnId"] = "native-turn"
                value.native_body.update(change)
                with self.assertRaisesRegex(RuntimeError, "^browser comparison failed$"):
                    await value.prepare_context()
                self.assertEqual(value.calls, [])

    async def test_wrapper_selection_is_explicit_and_validated_before_navigation(self):
        for change in ({"wrapper_url": "https://other.invalid/wrapper.mjs"},
                       {"wrapper_export": "a"}, {"wrapper_url": None}):
            value = self.fixture()
            with self.assertRaisesRegex(RuntimeError, "^browser comparison failed$"):
                await value.prepare_context(**change)
            self.assertEqual(value.calls, [])

    async def test_bad_body_shape_or_limit_is_rejected_before_page_use(self):
        for change in ({"castleToken": "private-forbidden"},
                       {"request": {"stream": True}},
                       {"request": {"stream": False, "input": "x" * C.COMPARISON_BYTES}},
                       {"extra": float("nan")}):
            value = self.fixture()
            value.body.update(change)
            with self.assertRaisesRegex(RuntimeError, "^browser comparison failed$"):
                await value.prepare_context()
            self.assertEqual(value.calls, [])

    async def test_phases_reject_out_of_order_replay_and_ordinary_mode(self):
        value = self.fixture()
        with self.assertRaises(ValueError):
            await value.command("browser_baseline_once")
        await value.prepare_context()
        with self.assertRaises(ValueError):
            await value.native()
        await value.baseline()
        with self.assertRaises(ValueError):
            await value.baseline()
        await value.native()
        for operation in (value.native, value.prepare_context):
            with self.assertRaises(ValueError):
                await operation()
        with self.assertRaises(ValueError):
            await value.command("ai_post")
        self.assertEqual((value.mints, value.fetches), (2, 1))

    async def test_each_leg_rejects_the_other_bound_envelope(self):
        value = self.fixture()
        await value.prepare_context()
        result = await value.baseline(body=value.native_body)
        self.assertFalse(result["accepted"])
        self.assertEqual((value.mints, value.fetches), (0, 0))
        value = self.fixture()
        await value.prepare_context()
        await value.baseline()
        with self.assertRaisesRegex(RuntimeError, "^browser comparison failed$"):
            await value.native(body=value.body)
        self.assertEqual((value.mints, value.fetches), (1, 1))

    async def test_403_reports_policy_and_blocks_native_without_private_content(self):
        value = self.fixture()
        await value.prepare_context()
        value.fetch_result = wire({"error": {"status": 403,
            "message": "This request was blocked by our security checks",
            "data": {"reason": "request_blocked"}, "ref": "private-fixture-ref",
            "timestamp": "private-fixture-time"}}, status=403)
        result = await value.baseline()
        self.assertFalse(result["accepted"])
        self.assertEqual(result["status"], 403)
        self.assertEqual(result["refusal"],
                         {"policy": "castle", "basis": "framer_security_envelope"})
        self.assertTrue(result["response_complete"] and result["fetch_settled"])
        self.assertEqual((result["transport_starts"], result["transport_finishes"]), (1, 1))
        serialized = json.dumps(result)
        for private in ("private-fixture", "fixture prompt", "blocked by", OLD.LOGIN, DEVICE):
            self.assertNotIn(private, serialized)
        with self.assertRaises(ValueError):
            await value.native()
        self.assertEqual((value.mints, value.fetches), (1, 1))

    async def test_provider_marker_and_fallback_remain_policy_evidence(self):
        for payload, elapsed, expected in (({"error": "provider refused"}, 1, "upstream"),
                                           ({"error": "ambiguous"}, 1200, "upstream"),
                                           ({"error": "ambiguous"}, 100, "castle")):
            value = self.fixture()
            await value.prepare_context()
            value.fetch_result = wire(payload, status=403, elapsed_ms=elapsed)
            result = await value.baseline()
            self.assertEqual(result["refusal"]["policy"], expected)
            self.assertFalse(result["accepted"])

    async def test_http_200_requires_complete_json_and_nonempty_text(self):
        cases = [wire({"error": "private error"}), wire(dict(response(), id="")),
                 wire(dict(response(), status="incomplete")), wire(dict(response(), output=[])),
                 wire(dict(response(), output=[{"type": "message", "content": [
                     {"type": "output_text", "text": "  "}]}])), wire(complete=False),
                 wire(content_type="text/event-stream"), wire(body="{", bytes=1)]
        for raw in cases:
            with self.subTest(raw=raw):
                value = self.fixture()
                await value.prepare_context()
                value.fetch_result = raw
                result = await value.baseline()
                self.assertFalse(result["accepted"])
                with self.assertRaises(ValueError):
                    await value.native()

    async def test_messages_route_requires_complete_nonempty_message(self):
        for stop_reason, text, accepted in (("end_turn", "answer", True),
                                            ("max_tokens", "partial", False),
                                            ("end_turn", "", False)):
            value = self.fixture()
            await value.prepare_context(path="/ai/v3/messages/")
            value.fetch_result = wire({"id": "fixture-message", "stop_reason": stop_reason,
                                       "content": [{"type": "text", "text": text}]})
            self.assertEqual((await value.baseline())["accepted"], accepted)

    async def test_invalid_or_duplicate_json_members_cannot_pass(self):
        for body in ('{"id":"x","id":"y"}', '{"id":NaN}', '{"id":"x"} trailing'):
            value = self.fixture()
            await value.prepare_context()
            value.fetch_result = wire(body=body, bytes=len(body.encode()))
            result = await value.baseline()
            self.assertFalse(result["accepted"])
            self.assertEqual(result["failure"], "invalid_json")

    async def test_device_is_observed_and_never_seeded(self):
        for rows in ([], [{"name": "__cuid", "value": "other-device"}],
                     [{"name": "__cuid", "value": DEVICE}] * 2):
            value = self.fixture()
            value.context.rows = [{"name": "session", "value": OLD.LOGIN}] + rows
            with self.assertRaisesRegex(RuntimeError, "^browser comparison failed$"):
                await value.prepare_context()
            writes = [call[1] for call in value.calls if call[0] == "add_cookies"]
            self.assertEqual(len(writes), 1)
            self.assertEqual([row["name"] for row in writes[0]], ["session"])
            self.assertEqual((value.mints, value.fetches), (0, 0))

    async def test_context_drift_is_refused_at_both_attempt_barriers(self):
        changes = {
            "device": lambda v: setattr(v.page, "device", "changed-device"),
            "cookie": lambda v: v.context.rows[1].update(value="changed-device"),
            "login": lambda v: v.context.rows[0].update(value="changed-login"),
            "page": lambda v: setattr(v.account, "page", object()),
            "context": lambda v: setattr(v.account, "ctx", object()),
            "browser": lambda v: setattr(v.account, "browser", object()),
            "scope": lambda v: setattr(v.scope, "scope_id", "changed-scope"),
            "generation": lambda v: setattr(v.scope, "generation", 8),
            "deadline": lambda v: v.scope.comparison.update(deadline=0),
            "navigation": lambda v: v.page.events["framenavigated"][0](v.page.main_frame),
        }
        for phase in ("baseline", "native"):
            for name, mutate in changes.items():
                with self.subTest(phase=phase, drift=name):
                    value = self.fixture()
                    await value.prepare_context()
                    if phase == "native":
                        await value.baseline()
                    mutate(value)
                    await self.assert_refused_before_next_mint(value, phase)

    async def assert_refused_before_next_mint(self, value, phase):
        before = value.mints, value.fetches
        if phase == "baseline":
            self.assertFalse((await value.baseline())["accepted"])
        else:
            with self.assertRaisesRegex(RuntimeError, "^browser comparison failed$"):
                await value.native()
        self.assertEqual((value.mints, value.fetches), before)

    async def test_identity_change_during_snapshot_is_rechecked(self):
        for phase in ("baseline", "native"):
            value = self.fixture()
            await value.prepare_context()
            if phase == "native":
                await value.baseline()
            value.context.on_cookies = lambda: setattr(value.scope, "generation", 8)
            await self.assert_refused_before_next_mint(value, phase)

    async def test_bearer_jar_must_be_current_before_baseline(self):
        value = self.fixture()
        await value.prepare_context()
        value.context.rows.append({"name": "another", "value": "current-value"})
        result = await value.baseline()
        self.assertFalse(result["accepted"])
        self.assertEqual((value.mints, value.fetches), (0, 0))

    async def test_native_returns_current_jar_for_fresh_bearer(self):
        value = self.fixture()
        await value.prepare_context()
        await value.baseline()
        value.context.rows.append({"name": "another", "value": "current-value"})
        result = await value.native()
        self.assertTrue(result["cookie_header"].endswith("another=current-value"))

    async def test_cancel_during_mint_cannot_start_baseline_fetch(self):
        value = self.fixture()
        await value.prepare_context()
        value.page.sdk_release.clear()
        task = asyncio.create_task(value.baseline())
        self.tasks.append(task)
        await asyncio.wait_for(value.page.sdk_started.wait(), 2)
        owned = value.scope.submitted[-1]
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertFalse(owned.done())
        self.assertEqual(value.scope.failure, "comparison_cancelled")
        value.page.sdk_release.set()
        await asyncio.wait_for(value.scope.drain(), 2)
        self.assertEqual(value.fetches, 0)
        with self.assertRaises(ValueError):
            await value.native()

    async def test_cancel_during_fetch_retains_owned_rpc_and_fencing(self):
        value = self.fixture()
        await value.prepare_context()
        value.fetch_release.clear()
        task = asyncio.create_task(value.baseline())
        self.tasks.append(task)
        await asyncio.wait_for(value.fetch_started.wait(), 2)
        owned = value.scope.submitted[-1]
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertFalse(owned.done())
        self.assertEqual(value.scope.comparison_phase, "failed")
        with self.assertRaises(ValueError):
            await value.native()

    async def test_unknown_fetch_failure_is_bounded_and_fenced(self):
        value = self.fixture()
        await value.prepare_context()
        value.fetch_error = True
        result = await value.baseline()
        self.assertFalse(result["accepted"] or result["fetch_settled"])
        self.assertIsNone(result["transport_starts"])
        self.assertIsNone(result["transport_finishes"])
        self.assertEqual(result["failure"], "unconfirmed")
        self.assertEqual(value.scope.failure, "comparison_fetch_unconfirmed")
        self.assertNotIn("private-fixture", json.dumps(result))
        with self.assertRaises(ValueError):
            await value.native()

    async def test_malformed_response_observations_do_not_establish_completion(self):
        for change in ({"bytes": 0}, {"finished": 0}, {"settled": False},
                       {"status": True}, {"started": 2}, {"failure": "private detail"}):
            value = self.fixture()
            await value.prepare_context()
            value.fetch_result.update(change)
            result = await value.baseline()
            self.assertFalse(result["accepted"])
            self.assertNotIn("private detail", json.dumps(result))
            with self.assertRaises(ValueError):
                await value.native()

    async def test_mint_error_is_sanitized_and_never_sends(self):
        value = self.fixture()
        await value.prepare_context()
        value.page.fail_sdk = True
        result = await value.baseline()
        self.assertEqual(result["failure"], "preparation")
        self.assertEqual(value.fetches, 0)
        self.assertNotIn("private-fixture", json.dumps(result))

    async def test_host_continuation_never_recreates_missing_or_nonresident_account(self):
        for absent in (False, True):
            value = self.fixture()
            await value.prepare_context()
            value.host._observed_command = lambda req: req
            value.host.lifecycles[value.account_id].phase = "absent"
            if absent:
                value.host.accounts.clear()
            entered = []

            @asynccontextmanager
            async def use_account(*args):
                entered.append(args)
                yield value.account

            value.host._use_account = use_account
            handle = OLD.host_method("handle", {"asyncio": asyncio,
                "emit": lambda *args, **kwargs: None})
            req = dict(value.scope.identity(), **value.bound_fields(value.body),
                       cmd="browser_baseline_once", bearer="private-fixture-bearer",
                       cookie_header=value.context_reply["cookie_header"])
            with patch.dict(sys.modules, {"contextual_authorization": C}):
                with self.assertRaisesRegex(RuntimeError, "^browser comparison failed$"):
                    await handle(value.host, req)
            self.assertEqual(entered, [])
            self.assertEqual((value.mints, value.fetches), (0, 0))

    async def test_ordinary_one_shot_keeps_original_wrapper_call(self):
        value = OLD.Harness()
        result = await value.prepare()
        await asyncio.wait_for(value.scope.drain(), 2)
        self.assertEqual(result["source"], "framer_wrapper_direct")
        self.assertIn("module.a()", value.calls[2][1])
        self.assertEqual(value.calls[2][2], OLD.SDK_URL)


class BrowserComparisonObserverTests(unittest.IsolatedAsyncioTestCase):
    asyncSetUp = OBSERVER.ObserverTests.asyncSetUp
    asyncTearDown = OBSERVER.ObserverTests.asyncTearDown
    send = OBSERVER.ObserverTests.send
    frame = OBSERVER.ObserverTests.frame
    opened = OBSERVER.ObserverTests.opened
    seal = OBSERVER.ObserverTests.seal

    async def exchange(self, scope, cmd, **fields):
        request_id = self.send(cmd, **scope.identity(), **fields)
        result = await self.frame(lambda value: value.get("id") == request_id)
        await asyncio.wait_for(scope.wait_commands(), 2)
        return result

    async def comparison(self):
        scope = await self.opened(with_browser=False)
        value = Harness()
        value.scope, value.account_id = scope, scope.account
        value.account._observed_scope = scope
        value.host.accounts = {scope.account: value.account}
        value.host._observed = self.owner

        async def handle(req):
            return await C.comparison_command(value.host, scope.account, req)

        self.host.handle = handle
        result = await self.exchange(scope, "prepare_inference_authorization",
            purpose="baseline_context", project=OLD.PROJECT, path="/ai/v3/responses/",
            body=value.body, native_body=value.native_body, cookie_header="session=" + OLD.LOGIN,
            wrapper_url=C.COMPARISON_WRAPPER, wrapper_export=C.COMPARISON_EXPORT)
        self.assertTrue(result["ok"])
        value.context_reply = result["result"]
        return scope, value

    async def test_real_observer_runs_sequence_and_joins_commands(self):
        scope, value = await self.comparison()
        self.assertEqual(scope.comparison_phase, "context_ready")
        result = await self.exchange(scope, "browser_baseline_once",
            **value.bound_fields(value.body), bearer="private-fixture-bearer",
            cookie_header=value.context_reply["cookie_header"])
        self.assertTrue(result["result"]["accepted"])
        self.assertEqual(scope.comparison_phase, "baseline_passed")
        result = await self.exchange(scope, "prepare_inference_authorization",
            purpose="native_token", **value.bound_fields(value.native_body))
        self.assertTrue(result["ok"])
        self.assertEqual(scope.comparison_phase, "complete")
        self.assertEqual((value.mints, value.fetches), (2, 1))
        self.assertFalse(scope.rpcs or scope.commands)
        self.assertEqual((await self.seal(scope))["outcome"], "closed")

    async def test_visible_reply_does_not_allow_work_before_previous_command_joins(self):
        scope, value = await self.comparison()
        release = asyncio.Event()
        self.gates.append(release)
        previous_send = self.writer.send
        baseline_id = self.sequence + 1

        async def send_with_pause(frame):
            await previous_send(frame)
            if frame.get("id") == baseline_id:
                await release.wait()

        self.writer.send = send_with_pause
        self.send("browser_baseline_once", **scope.identity(), **value.bound_fields(value.body),
            bearer="private-fixture-bearer", cookie_header=value.context_reply["cookie_header"])
        result = await self.frame(lambda item: item.get("id") == baseline_id)
        self.assertTrue(result["result"]["accepted"])
        native_id = self.send("prepare_inference_authorization", **scope.identity(),
            purpose="native_token", **value.bound_fields(value.native_body))
        await asyncio.sleep(0)
        self.assertEqual((value.mints, value.fetches), (1, 1))
        self.assertEqual(len(scope.commands), 2)
        release.set()
        result = await self.frame(lambda item: item.get("id") == native_id)
        await asyncio.wait_for(scope.wait_commands(), 2)
        self.assertTrue(result["ok"])
        self.assertEqual((value.mints, value.fetches), (2, 1))

    async def test_real_observer_refuses_native_after_baseline_failure(self):
        scope, value = await self.comparison()
        value.fetch_result = wire({"error": "Forbidden"}, status=403)
        result = await self.exchange(scope, "browser_baseline_once",
            **value.bound_fields(value.body), bearer="private-fixture-bearer",
            cookie_header=value.context_reply["cookie_header"])
        self.assertFalse(result["result"]["accepted"])
        self.assertEqual(scope.comparison_phase, "failed")
        result = await self.exchange(scope, "prepare_inference_authorization",
            purpose="native_token", **value.bound_fields(value.native_body))
        self.assertFalse(result["ok"])
        self.assertEqual((value.mints, value.fetches), (1, 1))
        self.assertEqual((await self.seal(scope))["outcome"], "unconfirmed")

    async def test_preceding_reply_failure_retains_fencing_and_hides_raw_error(self):
        scope, value = await self.comparison()
        release = asyncio.Event()
        self.gates.append(release)
        previous_send = self.writer.send
        baseline_id = self.sequence + 1

        async def failing_send(frame):
            await previous_send(frame)
            if frame.get("id") == baseline_id:
                await release.wait()
                raise RuntimeError("private-fixture-writer-error")

        self.writer.send = failing_send
        self.send("browser_baseline_once", **scope.identity(), **value.bound_fields(value.body),
            bearer="private-fixture-bearer", cookie_header=value.context_reply["cookie_header"])
        await self.frame(lambda item: item.get("id") == baseline_id)
        native_id = self.send("prepare_inference_authorization", **scope.identity(),
            purpose="native_token", **value.bound_fields(value.native_body))
        release.set()
        result = await self.frame(lambda item: item.get("id") == native_id)
        await asyncio.wait_for(scope.wait_commands(), 2)
        self.assertFalse(result["ok"])
        self.assertNotIn("private-fixture", json.dumps(result))
        self.assertEqual((value.mints, value.fetches), (1, 1))
        self.assertTrue(scope.failure)
        self.assertEqual((await self.seal(scope))["outcome"], "unconfirmed")


if __name__ == "__main__":
    unittest.main(verbosity=2)
