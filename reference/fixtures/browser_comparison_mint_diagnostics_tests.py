"""Inert mint diagnostics, UNRUN by the source-only author.

These fake RPC replies exercise Python ownership and privacy. The runtime
owner separately probes the exact JavaScript; no SDK or browser runs here.
"""
import asyncio
import builtins
import importlib.util
import json
from pathlib import Path
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "_mint_diagnostic_fixture", Path(__file__).with_name("browser_comparison_diagnostics_tests.py"))
D = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(D)
F, C = D.FIXTURE, D.C
SECRET = D.SECRET
UNSET = object()
STAGES = {"pre_context": "mint_pre_context", "dynamic_import": "mint_import",
          "export_validation": "mint_export", "wrapper_call": "mint_wrapper_call",
          "post_context": "mint_post_context", "post_page": "mint_post_page",
          "post_device": "mint_post_device", "invalid_token": "mint_invalid_token"}


class PrivateRPCError(RuntimeError):
    def __str__(self):
        raise AssertionError("RPC exceptions must not be rendered")


class TargetClosedError(PrivateRPCError):
    pass


class TimeoutError(PrivateRPCError):
    pass


class ComparisonMintDiagnosticsTests(unittest.IsolatedAsyncioTestCase):
    asyncSetUp = F.BrowserComparisonTests.asyncSetUp
    asyncTearDown = F.BrowserComparisonTests.asyncTearDown
    fixture = D.ComparisonDiagnosticWorkflowTests.fixture
    assert_public_events = D.ComparisonDiagnosticWorkflowTests.assert_public_events
    assert_pre_send_failure = D.ComparisonDiagnosticWorkflowTests.assert_pre_send_failure

    def mint_reply(self, value, reply=UNSET, error=None, after=None):
        evaluate = value.page.evaluate
        value.mint_inputs = []

        async def controlled(script, args=None):
            result = await evaluate(script, args)
            if script == "mw:" + C.COMPARISON_MINT:
                value.mint_inputs.append(args)
                if after is not None:
                    after()
                if error is not None:
                    raise error
                return result if reply is UNSET else reply
            return result

        value.page.evaluate = controlled

    def success_reply(self, value, token):
        return {"before": value.page.url, "after": value.page.url, "token": token}

    async def test_js_stages_refuse_baseline_and_forbid_native_continuation(self):
        for failure, stage in STAGES.items():
            with self.subTest(stage=stage):
                value = self.fixture()
                await value.prepare_context()
                self.mint_reply(value, {"failure_stage": failure})
                await self.assert_pre_send_failure(value, stage, 1)
                with self.assertRaises(ValueError):
                    await value.native()
                self.assertEqual((value.mints, value.fetches), (1, 0))

    async def test_js_stages_keep_native_failure_generic_without_another_fetch(self):
        for failure, stage in STAGES.items():
            with self.subTest(stage=stage):
                value = self.fixture()
                await value.prepare_context()
                self.assertTrue((await value.baseline())["accepted"])
                self.mint_reply(value, {"failure_stage": failure})
                with self.assertRaisesRegex(RuntimeError, "^browser comparison failed$") as caught:
                    await value.native()
                self.assertIsNone(caught.exception.__cause__)
                self.assertTrue(caught.exception.__suppress_context__)
                self.assertEqual((value.mints, value.fetches), (2, 1))
                self.assertEqual(value.scope.comparison_phase, "failed")
                self.assertEqual(value.events[-1], dict(event="browser.comparison.command",
                                                      outcome="failed", stage=stage))
                self.assert_public_events(value)

    async def test_malformed_replies_hide_unknown_stages_and_private_fields(self):
        replies = [None, [], SECRET, {}, {"failure_stage": SECRET},
                   *({"failure_stage": item} for item in (True, 1, [], {})),
                   {"failure_stage": "wrapper_call", "token": SECRET},
                   {"failure_stage": "wrapper_call", "stack": SECRET},
                   {"before": SECRET, "after": SECRET, "token": SECRET, "extra": SECRET}]
        for index, reply in enumerate(replies):
            with self.subTest(case=index):
                value = self.fixture()
                await value.prepare_context()
                self.mint_reply(value, reply)
                await self.assert_pre_send_failure(value, "mint_reply_invalid", 1)

    async def test_success_envelope_context_drift_still_refuses(self):
        for field, stage in (("before", "mint_pre_context"), ("after", "mint_post_context")):
            with self.subTest(field=field):
                value = self.fixture()
                await value.prepare_context()
                reply = self.success_reply(value, SECRET)
                reply[field] = "https://framer.com/projects/" + SECRET
                self.mint_reply(value, reply)
                await self.assert_pre_send_failure(value, stage, 1)

    async def test_invalid_tokens_keep_strict_utf8_and_byte_limit(self):
        tokens = (None, False, 1, "", "x" * (C.LIMIT + 1),
                  "\u00e9" * (C.LIMIT // 2 + 1), "\ud800", "\udfff")
        for index, token in enumerate(tokens):
            with self.subTest(case=index):
                value = self.fixture()
                await value.prepare_context()
                self.mint_reply(value, self.success_reply(value, token))
                await self.assert_pre_send_failure(value, "mint_invalid_token", 1)

    async def test_valid_utf8_token_at_limit_is_returned_unchanged(self):
        for token in ("x" * C.LIMIT, "\u00e9" * (C.LIMIT // 2)):
            value = self.fixture()
            await value.prepare_context()
            self.mint_reply(value, self.success_reply(value, token))
            actual = await C.comparison_mint(value.scope, value.scope.comparison, value.account)
            self.assertEqual(actual, token)
            self.assertEqual((value.mints, value.fetches), (1, 0))
            self.assertEqual(C.comparison_evidence(value.scope), {"stage": "mint_rpc"})

    async def test_valid_pair_keeps_exact_wrapper_page_and_device_and_two_fresh_mints(self):
        value = self.fixture()
        await value.prepare_context()
        self.mint_reply(value)
        self.assertTrue((await value.baseline())["accepted"])
        native = await value.native()
        self.assertEqual((value.mints, value.fetches), (2, 1))
        self.assertNotEqual(native["token"], value.fetch_args["token"])
        expected = {"url": C.COMPARISON_WRAPPER, "entry": C.COMPARISON_EXPORT,
                    "device": F.DEVICE, "page_url": F.OLD.PAGE_URL}
        self.assertEqual(value.mint_inputs, [expected, expected])
        self.assertEqual(value.scope.comparison_phase, "complete")
        self.assertEqual([call[0] for call in value.calls].count("goto"), 1)
        self.assertEqual([call[0] for call in value.calls].count("add_cookies"), 1)
        self.assert_public_events(value)

    async def test_outer_rpc_categories_rethrow_the_same_exception_without_rendering(self):
        cases = ((TargetClosedError, "mint_rpc_target_closed"),
                 (TimeoutError, "mint_rpc_timeout"), (builtins.TimeoutError, "mint_rpc_timeout"),
                 (type(SECRET, (PrivateRPCError,), {}), "mint_rpc_error"))
        for kind, stage in cases:
            with self.subTest(stage=stage):
                value = self.fixture()
                await value.prepare_context()
                error = kind(SECRET)
                self.mint_reply(value, error=error)
                with self.assertRaises(kind) as caught:
                    await C.comparison_mint(value.scope, value.scope.comparison, value.account)
                self.assertIs(caught.exception, error)
                self.assertEqual(C.comparison_evidence(value.scope), {"stage": stage})
                self.assertEqual((value.mints, value.fetches), (1, 0))

    async def test_baseline_rpc_failure_retains_preparation_result_and_private_events(self):
        for kind, stage in ((TargetClosedError, "mint_rpc_target_closed"),
                            (TimeoutError, "mint_rpc_timeout"), (PrivateRPCError, "mint_rpc_error")):
            with self.subTest(stage=stage):
                value = self.fixture()
                await value.prepare_context()
                self.mint_reply(value, error=kind(SECRET))
                await self.assert_pre_send_failure(value, stage, 1)
                with self.assertRaises(ValueError):
                    await value.native()
                self.assertEqual((value.mints, value.fetches), (1, 0))

    async def test_native_rpc_failure_stays_generic_and_never_repeats_baseline(self):
        value = self.fixture()
        await value.prepare_context()
        self.assertTrue((await value.baseline())["accepted"])
        self.mint_reply(value, error=TargetClosedError(SECRET))
        with self.assertRaisesRegex(RuntimeError, "^browser comparison failed$"):
            await value.native()
        self.assertEqual(value.events[-1], dict(event="browser.comparison.command",
            outcome="failed", stage="mint_rpc_target_closed"))
        self.assertEqual((value.mints, value.fetches), (2, 1))
        self.assertEqual(value.scope.comparison_phase, "failed")
        self.assert_public_events(value)

    async def test_existing_scope_failure_is_retained_without_exporting_its_reason(self):
        value = self.fixture()
        await value.prepare_context()
        self.mint_reply(value, after=lambda: value.scope.fail(SECRET))
        await self.assert_pre_send_failure(value, "mint_rpc_error", 1)
        self.assertEqual(value.scope.failure, SECRET)
        self.assertNotIn(SECRET, json.dumps(C.comparison_evidence(value.scope)))

    async def test_native_cancellation_keeps_pending_rpc_and_does_not_mark_rpc_error(self):
        value = self.fixture()
        await value.prepare_context()
        self.assertTrue((await value.baseline())["accepted"])
        value.page.sdk_started.clear()
        value.page.sdk_release.clear()
        task = asyncio.create_task(value.native())
        self.tasks.append(task)
        await asyncio.wait_for(value.page.sdk_started.wait(), 1)
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertEqual(C.comparison_evidence(value.scope), {"stage": "mint_rpc"})
        self.assertTrue(value.scope.pending)
        self.assertEqual(len(value.events), 2)
        self.assertEqual(value.scope.comparison_phase, "failed")
        self.assertEqual((value.mints, value.fetches), (2, 1))
        value.page.sdk_release.set()
        await asyncio.wait_for(value.scope.drain(), 1)
        self.assertFalse(value.scope.pending)
        self.assertEqual((value.mints, value.fetches), (2, 1))
        self.assert_public_events(value)

    async def test_rpc_deadline_leaves_mint_owned_until_its_late_completion(self):
        value = self.fixture()
        await value.prepare_context()
        value.page.sdk_release.clear()
        original = C.comparison_rpc

        async def bounded(scope, make, timeout=45):
            return await original(scope, make, timeout=0.05)

        with patch.object(C, "comparison_rpc", bounded):
            with self.assertRaises(builtins.TimeoutError):
                await C.comparison_mint(value.scope, value.scope.comparison, value.account)
        self.assertTrue(value.page.sdk_started.is_set())
        self.assertEqual(C.comparison_evidence(value.scope), {"stage": "mint_rpc_timeout"})
        self.assertTrue(value.scope.pending)
        self.assertEqual((value.mints, value.fetches), (1, 0))
        value.page.sdk_release.set()
        await asyncio.wait_for(value.scope.drain(), 1)
        self.assertFalse(value.scope.pending)
        self.assertEqual((value.mints, value.fetches), (1, 0))


if __name__ == "__main__":
    unittest.main()
