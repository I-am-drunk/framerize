"""Inert diagnostic fixtures; UNRUN by the source-only author.

Reuses the comparison fakes and extracts the actual Host.handle method.
Only the runtime owner runs these cases. A returned command event is not
evidence of accepted inference, delivery or cleanup.
"""
import asyncio
from contextlib import asynccontextmanager
import importlib.util
import json
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location(
    "_comparison_diagnostic_fixture", Path(__file__).with_name("browser_comparison_tests.py"))
FIXTURE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(FIXTURE)
C = FIXTURE.C
SECRET = "private-diagnostic-sentinel"
CHECKS = ("device_valid", "device_cookie_match", "login_match", "page_match", "url_unchanged",
          "device_cookie_present", "device_cookie_unique")
BASELINE_STAGES = ("baseline_header_before_mint", "baseline_mint", "baseline_bound_after_mint",
                   "baseline_header_after_mint", "baseline_fetch_args", "baseline_fetch",
                   "baseline_result", "baseline_bound_after_fetch")
STAGES = {"account_start", "context_inputs", "cookie_seed", "navigation", "device_readiness", "snapshot_page",
          "snapshot_cookies", "snapshot_header", "snapshot_validate", "context_ready",
          "baseline", "native_token", *BASELINE_STAGES,
          "mint_rpc", "mint_rpc_target_closed", "mint_rpc_timeout", "mint_rpc_error",
          "mint_reply_invalid", "mint_pre_context", "mint_import", "mint_export",
          "mint_wrapper_call", "mint_post_context", "mint_invalid_token",
          "baseline_header_before_mint_order", "baseline_header_before_mint_membership",
          "baseline_header_before_mint_values", "baseline_header_after_mint_order",
          "baseline_header_after_mint_membership", "baseline_header_after_mint_values"}


class DiagnosticHarness(FIXTURE.Harness):
    def __init__(self):
        super().__init__()
        self.events = []
        self.host._observed_command = lambda request: request
        self.host.lifecycles[self.account_id].phase = "resident"
        self.host._use_account = self.use_account
        self.handle = FIXTURE.OLD.host_method("handle", {"asyncio": asyncio, "emit": self.emit})

    def emit(self, event, **fields):
        self.events.append(dict(event=event, **fields))

    @asynccontextmanager
    async def use_account(self, account, proxy):
        assert account == self.account_id and proxy is None
        yield self.account

    async def command(self, cmd, **fields):
        request = dict(self.scope.identity(), cmd=cmd, **fields)
        C.reserve_comparison(self.scope, request)
        self.scope.commands[1] = True
        try:
            with patch.dict(sys.modules, {"contextual_authorization": C}):
                return await self.handle(self.host, request)
        finally:
            self.scope.commands.clear()
            self.scope.comparison_phase = self.scope.comparison_next
            self.scope.comparison_next = "failed"


class ComparisonEvidenceTests(unittest.TestCase):
    def test_mark_filters_nonbooleans_and_clears_previous_checks(self):
        scope = SimpleNamespace()
        C.comparison_mark(scope, "snapshot_validate", device_valid=False,
                          device_cookie_match=1, login_match=True, page_match="true",
                          url_unchanged=None, token=SECRET, exception=SECRET)
        self.assertEqual(C.comparison_evidence(scope), {
            "stage": "snapshot_validate", "device_valid": False, "login_match": True})
        C.comparison_mark(scope, "cookie_seed")
        self.assertEqual(C.comparison_evidence(scope), {"stage": "cookie_seed"})
        C.comparison_mark(scope, SECRET, device_valid=True)
        self.assertEqual(C.comparison_evidence(scope), {"stage": "unknown", "device_valid": True})

    def test_evidence_keeps_only_the_five_exact_boolean_fields(self):
        booleans = {key: index % 2 == 0 for index, key in enumerate(CHECKS)}
        private = {key: SECRET for key in ("token", "cookie_header", "device", "page_url",
                                          "content", "exception", "unknown")}
        scope = SimpleNamespace(comparison_diagnostic_stage="snapshot_validate",
                                comparison_diagnostic_checks=dict(private, **booleans))
        self.assertEqual(C.comparison_evidence(scope), {"stage": "snapshot_validate", **booleans})
        for value in (0, 1, 0.0, 1.0, None, "true", SECRET, [], {}, object()):
            with self.subTest(type=type(value).__name__):
                scope.comparison_diagnostic_checks = dict(private, **dict.fromkeys(CHECKS, value))
                self.assertEqual(C.comparison_evidence(scope), {"stage": "snapshot_validate"})

    def test_evidence_rejects_unknown_stages_and_nonplain_check_mappings(self):
        class StringSubclass(str):
            pass

        class DictSubclass(dict):
            pass

        self.assertEqual(C.comparison_evidence(None), {"stage": "unknown"})
        scope = SimpleNamespace(comparison_diagnostic_checks={"device_valid": False})
        for stage in (None, SECRET, True, [], {}, b"baseline", StringSubclass("native_token")):
            scope.comparison_diagnostic_stage = stage
            self.assertEqual(C.comparison_evidence(scope), {"stage": "unknown", "device_valid": False})
        for stage in STAGES:
            scope.comparison_diagnostic_stage = stage
            self.assertEqual(C.comparison_evidence(scope), {"stage": stage, "device_valid": False})
        for checks in (None, [], SECRET, SimpleNamespace(device_valid=True),
                       DictSubclass(device_valid=True)):
            scope.comparison_diagnostic_stage = "snapshot_header"
            scope.comparison_diagnostic_checks = checks
            self.assertEqual(C.comparison_evidence(scope), {"stage": "snapshot_header"})


class ComparisonDiagnosticWorkflowTests(unittest.IsolatedAsyncioTestCase):
    asyncSetUp = FIXTURE.BrowserComparisonTests.asyncSetUp
    asyncTearDown = FIXTURE.BrowserComparisonTests.asyncTearDown

    def fixture(self):
        value = DiagnosticHarness()
        self.values.append(value)
        return value

    def assert_public_events(self, value):
        for event in value.events:
            self.assertEqual(event["event"], "browser.comparison.command")
            self.assertIn(event["outcome"], ("returned", "failed"))
            self.assertIn(event["stage"], STAGES | {"unknown"})
            self.assertLessEqual(set(event), {"event", "outcome", "stage", *CHECKS})
            for key in set(event) & set(CHECKS):
                self.assertIs(type(event[key]), bool)
        encoded = json.dumps(value.events)
        for private in (SECRET, FIXTURE.OLD.LOGIN, FIXTURE.DEVICE, FIXTURE.OLD.PROJECT,
                        "private-fixture", "fixture prompt", "fixture answer"):
            self.assertNotIn(private, encoded)

    async def assert_context_failure(self, value, stage, **checks):
        with self.assertRaisesRegex(RuntimeError, "^browser comparison failed$") as caught:
            await value.prepare_context()
        self.assertIsNone(caught.exception.__cause__)
        self.assertTrue(caught.exception.__suppress_context__)
        self.assertEqual(value.events, [dict(event="browser.comparison.command",
                                            outcome="failed", stage=stage, **checks)])
        self.assertEqual((value.mints, value.fetches), (0, 0))
        self.assertEqual(value.scope.comparison_phase, "failed")
        self.assert_public_events(value)

    async def assert_pre_send_failure(self, value, stage, mints):
        result = await value.baseline()
        self.assertEqual(result["failure"], "preparation")
        self.assertFalse(result["accepted"])
        self.assertIsNone(result["status"])
        self.assertEqual((result["body_bytes"], result["transport_starts"],
                          result["transport_finishes"]), (0, 0, 0))
        self.assertTrue(result["fetch_settled"])
        self.assertEqual((value.mints, value.fetches), (mints, 0))
        self.assertEqual(value.scope.comparison_phase, "failed")
        self.assertEqual(value.events[-1], dict(event="browser.comparison.command",
                                              outcome="returned", stage=stage))
        self.assert_public_events(value)

    def change_after_mint(self, value, change):
        evaluate = value.page.evaluate

        async def changed(script, args=None):
            result = await evaluate(script, args)
            if script == C.COMPARISON_MINT:
                change()
            return result

        value.page.evaluate = changed

    async def test_baseline_first_header_drift_reports_stage_before_any_mint(self):
        value = self.fixture()
        await value.prepare_context()
        value.context.rows.append({"name": "private_cookie", "value": SECRET})
        await self.assert_pre_send_failure(value, "baseline_header_before_mint_membership", 0)

    async def test_baseline_mint_error_reports_stage_without_private_exception(self):
        value = self.fixture()
        await value.prepare_context()
        value.page.fail_sdk = True
        await self.assert_pre_send_failure(value, "mint_rpc_error", 1)

    async def test_baseline_after_mint_header_drift_reports_separate_stage(self):
        value = self.fixture()
        await value.prepare_context()
        self.change_after_mint(value, lambda: value.context.rows.append(
            {"name": "private_cookie", "value": SECRET}))
        await self.assert_pre_send_failure(value, "baseline_header_after_mint_membership", 1)

    async def test_baseline_binding_refusal_after_mint_is_distinct_from_header_drift(self):
        value = self.fixture()
        await value.prepare_context()
        self.change_after_mint(value, lambda: setattr(value.scope, "generation", 8))
        await self.assert_pre_send_failure(value, "baseline_bound_after_mint", 1)

    async def test_baseline_fetch_rpc_error_keeps_unknown_outcome_and_stage(self):
        value = self.fixture()
        await value.prepare_context()
        value.fetch_error = True
        result = await value.baseline()
        self.assertEqual(result["failure"], "unconfirmed")
        self.assertFalse(result["accepted"] or result["fetch_settled"])
        self.assertIsNone(result["transport_starts"])
        self.assertIsNone(result["transport_finishes"])
        self.assertEqual((value.mints, value.fetches), (1, 1))
        self.assertEqual(value.scope.failure, "comparison_fetch_unconfirmed")
        self.assertEqual(value.events[-1], dict(event="browser.comparison.command",
                                              outcome="returned", stage="baseline_fetch"))
        self.assert_public_events(value)

    async def test_baseline_malformed_result_reports_stage_and_preserves_fencing(self):
        value = self.fixture()
        await value.prepare_context()
        value.fetch_result = {"token": SECRET, "exception": SECRET}
        result = await value.baseline()
        self.assertEqual(result["failure"], "protocol")
        self.assertFalse(result["accepted"] or result["fetch_settled"])
        self.assertEqual((value.mints, value.fetches), (1, 1))
        self.assertEqual(value.scope.failure, "comparison_fetch_unconfirmed")
        self.assertEqual(value.events[-1], dict(event="browser.comparison.command",
                                              outcome="returned", stage="baseline_result"))
        self.assertNotIn(SECRET, json.dumps(result))
        self.assert_public_events(value)

    async def test_baseline_cancelled_mint_keeps_stage_and_owned_pending_rpc(self):
        value = self.fixture()
        await value.prepare_context()
        value.page.sdk_release.clear()
        task = asyncio.create_task(value.baseline())
        self.tasks.append(task)
        await asyncio.wait_for(value.page.sdk_started.wait(), 1)
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertEqual(C.comparison_evidence(value.scope), {"stage": "mint_rpc"})
        self.assertEqual(value.scope.failure, "comparison_cancelled")
        self.assertTrue(value.scope.pending)
        self.assertEqual((value.mints, value.fetches), (1, 0))
        self.assertEqual(len(value.events), 1)
        value.page.sdk_release.set()
        await asyncio.wait_for(value.scope.drain(), 1)
        self.assertFalse(value.scope.pending)
        self.assertEqual(value.fetches, 0)
        self.assert_public_events(value)

    async def test_missing_device_reports_snapshot_validation_flags(self):
        value = self.fixture()

        async def ready_then_disappears(script, **options):
            handle = await FIXTURE.Harness.wait_for_function(value, script, **options)
            value.page.device = None
            return handle

        value.page.wait_for_function = ready_then_disappears
        checks = dict.fromkeys(CHECKS, True)
        checks.update(device_valid=False, device_cookie_match=False)
        await self.assert_context_failure(value, "snapshot_validate", **checks)

    async def test_page_mismatch_reports_snapshot_validation_flags(self):
        value = self.fixture()
        value.page.goto_result = "https://framer.com/projects/another-project-" + SECRET
        checks = dict.fromkeys(CHECKS, True)
        checks["page_match"] = False
        await self.assert_context_failure(value, "snapshot_validate", **checks)

    async def test_absent_api_device_cookie_is_distinct_from_mismatch(self):
        value = self.fixture()
        value.context.rows = value.context.rows[:1]
        checks = dict.fromkeys(CHECKS, True)
        checks.update(device_cookie_match=False, device_cookie_present=False,
                      device_cookie_unique=False)
        await self.assert_context_failure(value, "snapshot_validate", **checks)

    async def test_duplicate_api_device_cookie_is_distinct_from_mismatch(self):
        value = self.fixture()
        value.context.rows.append(dict(value.context.rows[1]))
        checks = dict.fromkeys(CHECKS, True)
        checks.update(device_cookie_match=False, device_cookie_unique=False)
        await self.assert_context_failure(value, "snapshot_validate", **checks)

    async def test_different_api_device_cookie_reports_no_cookie_value(self):
        value = self.fixture()
        value.context.rows[1]["value"] = SECRET
        checks = dict.fromkeys(CHECKS, True)
        checks.update(device_cookie_match=False)
        await self.assert_context_failure(value, "snapshot_validate", **checks)

    async def test_header_failure_has_no_stale_validation_flags(self):
        value = self.fixture()
        C.comparison_mark(value.scope, "snapshot_validate", **dict.fromkeys(CHECKS, True))
        value.scope.comparison_diagnostic_checks["cookie_header"] = SECRET
        value.context.rows.append({"name": "private", "value": SECRET + "\n"})
        await self.assert_context_failure(value, "snapshot_header")

    async def test_host_exception_is_generic_and_does_not_expose_supplied_secret(self):
        value = self.fixture()

        @asynccontextmanager
        async def failed_account(*args):
            raise RuntimeError(SECRET)
            yield

        value.host._use_account = failed_account
        await self.assert_context_failure(value, "account_start")

    async def test_host_validation_failure_uses_unknown_stage_without_private_input(self):
        value = self.fixture()
        value.scope.comparison_diagnostic_stage = SECRET
        with self.assertRaisesRegex(RuntimeError, "^browser comparison failed$"):
            await value.prepare_context(wrapper_url="https://" + SECRET + ".invalid/wrapper")
        self.assertEqual(value.events, [{"event": "browser.comparison.command",
                                        "outcome": "failed", "stage": "unknown"}])
        self.assertEqual(value.calls, [])
        self.assertEqual((value.mints, value.fetches), (0, 0))
        self.assert_public_events(value)

    async def test_valid_sequence_keeps_one_baseline_two_mints_and_private_events(self):
        value = self.fixture()
        context = await value.prepare_context()
        self.assertEqual((value.mints, value.fetches), (0, 0))
        with patch.object(C, "comparison_mark", wraps=C.comparison_mark) as marks:
            baseline = await value.baseline()
        stages = [call.args[1] for call in marks.call_args_list]
        self.assertEqual([stage for stage in stages if stage.startswith("baseline")],
                         ["baseline", *BASELINE_STAGES])
        self.assertTrue(baseline["accepted"] and baseline["complete_json"] and baseline["nonempty_text"])
        self.assertEqual((value.mints, value.fetches), (1, 1))
        native = await value.native()
        self.assertEqual((value.mints, value.fetches), (2, 1))
        self.assertEqual(native["context_handle"], context["context_handle"])
        self.assertNotEqual(native["token"], value.fetch_args["token"])
        self.assertEqual(value.scope.comparison_phase, "complete")
        common = {"event": "browser.comparison.command", "outcome": "returned"}
        validated = dict(common, stage="snapshot_validate", **dict.fromkeys(CHECKS, True))
        self.assertEqual(value.events, [dict(common, stage="context_ready"), validated, validated])
        names = [call[0] for call in value.calls]
        self.assertEqual(names.count("goto"), 1)
        self.assertEqual(names.count("add_cookies"), 1)
        self.assert_public_events(value)
        for private in (native["token"], value.fetch_args["token"], context["cookie_header"]):
            self.assertNotIn(private, json.dumps(value.events))

    async def test_returned_refusal_event_does_not_claim_baseline_success(self):
        value = self.fixture()
        await value.prepare_context()
        value.fetch_result = FIXTURE.wire({"error": SECRET}, status=403)
        result = await value.baseline()
        self.assertFalse(result["accepted"])
        self.assertEqual((value.mints, value.fetches), (1, 1))
        self.assertEqual(value.scope.comparison_phase, "failed")
        self.assertEqual(value.events[-1], dict(
            event="browser.comparison.command", outcome="returned",
            stage="baseline_result"))
        self.assert_public_events(value)


if __name__ == "__main__":
    unittest.main()
