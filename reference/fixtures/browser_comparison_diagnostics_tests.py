"""UNRUN diagnostic fixtures based on 4c94803; no browser or SDK execution.

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
CHECKS = ("device_valid", "device_cookie_match", "login_match", "page_match", "url_unchanged")
STAGES = {"account_start", "context_inputs", "cookie_seed", "navigation", "device_readiness", "snapshot_page",
          "snapshot_cookies", "snapshot_header", "snapshot_validate", "context_ready",
          "baseline", "native_token"}


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
        baseline = await value.baseline()
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
            stage="snapshot_validate", **dict.fromkeys(CHECKS, True)))
        self.assert_public_events(value)


if __name__ == "__main__":
    unittest.main()
