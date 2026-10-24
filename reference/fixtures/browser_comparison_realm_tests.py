"""Inert comparison realm fixtures. UNRUN by the source-only author."""
import importlib.util
import json
from pathlib import Path
import unittest

SPEC = importlib.util.spec_from_file_location(
    "_comparison_realm_fixture", Path(__file__).with_name("browser_comparison_tests.py"))
F = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(F)
C = F.C


class RealmHarness(F.Harness):
    def __init__(self):
        super().__init__()
        self.evaluations = []

    async def evaluate(self, script, args=None):
        self.evaluations.append((script, args))
        self.calls.append(("realm_evaluate", script))
        return await super().evaluate(script, args)


class BrowserComparisonRealmTests(unittest.IsolatedAsyncioTestCase):
    asyncSetUp = F.BrowserComparisonTests.asyncSetUp
    asyncTearDown = F.BrowserComparisonTests.asyncTearDown

    def fixture(self):
        value = RealmHarness()
        self.values.append(value)
        return value

    async def test_reserved_comparison_opts_in_before_browser_creation(self):
        value = self.fixture()
        C.reserve_comparison(value.scope, {"cmd": "prepare_inference_authorization",
                                           "purpose": "baseline_context"})
        self.assertEqual(value.scope.comparison_phase, "context_pending")
        _, _, launches, _ = await F.OLD.launch_fixture(
            True, comparison_phase=value.scope.comparison_phase)
        self.assertEqual(len(launches), 1)
        self.assertIs(launches[0]["main_world_eval"], True)
        self.assertEqual(launches[0]["exclude_addons"], [F.OLD.DEFAULT_UBO])
        _, _, ordinary, _ = await F.OLD.launch_fixture(True)
        self.assertEqual({key: item for key, item in launches[0].items()
                          if key not in ("main_world_eval", "exclude_addons")}, ordinary[0])

    async def test_ordinary_observed_and_unmanaged_keep_default_realm(self):
        for observed, phase in ((True, None), (False, None), (False, "context_pending")):
            with self.subTest(observed=observed, phase=phase):
                _, _, launches, _ = await F.OLD.launch_fixture(
                    observed, comparison_phase=phase)
                self.assertNotIn("main_world_eval", launches[0])
                self.assertNotIn("exclude_addons", launches[0])

    async def test_pair_uses_main_world_for_snapshots_both_mints_and_fetch(self):
        value = self.fixture()
        original = (value.account.page, value.account.ctx, value.account.browser)
        await value.prepare_context()
        self.assertTrue((await value.baseline())["accepted"])
        await value.native()
        scripts = [script for script, _ in value.evaluations]
        self.assertEqual(set(scripts), {"mw:" + C.COMPARISON_SNAPSHOT,
            "mw:" + C.COMPARISON_MINT, "mw:" + C.COMPARISON_FETCH})
        self.assertGreater(scripts.count("mw:" + C.COMPARISON_SNAPSHOT), 1)
        self.assertEqual(scripts.count("mw:" + C.COMPARISON_MINT), 2)
        self.assertEqual(scripts.count("mw:" + C.COMPARISON_FETCH), 1)
        for script, args in value.evaluations:
            if script == "mw:" + C.COMPARISON_MINT:
                self.assertEqual(args, {"url": C.COMPARISON_WRAPPER,
                    "entry": C.COMPARISON_EXPORT, "device": F.DEVICE,
                    "page_url": F.OLD.PAGE_URL, "observe_sdk": True})
        self.assertEqual(json.loads(value.fetch_args["body"]), value.body)
        self.assertEqual(value.fetch_args["device"], F.DEVICE)
        self.assertEqual(value.fetch_args["page_url"], F.OLD.PAGE_URL)
        self.assertEqual((value.account.page, value.account.ctx, value.account.browser), original)
        self.assertEqual((value.mints, value.fetches), (2, 1))

    async def test_readiness_keeps_plain_predicate_and_disposes_before_snapshot(self):
        value = self.fixture()
        await value.prepare_context()
        ready = [call for call in value.calls if call[0] == "wait_for_function"]
        self.assertEqual(ready, [("wait_for_function", C.COMPARISON_READY, 10000, 100,
                                 {'project': F.OLD.PROJECT})])
        self.assertFalse(ready[0][1].startswith("mw:"))
        names = [call[0] for call in value.calls]
        self.assertLess(names.index("goto"), names.index("wait_for_function"))
        self.assertLess(names.index("dispose_readiness"), names.index("realm_evaluate"))
        self.assertEqual(value.evaluations, [("mw:" + C.COMPARISON_SNAPSHOT, None)])
        seeds = [call[1] for call in value.calls if call[0] == "add_cookies"]
        self.assertEqual([[row["name"] for row in rows] for rows in seeds], [["session"]])
        self.assertEqual((value.mints, value.fetches), (0, 0))

    async def test_other_comparison_phases_do_not_select_launch_option(self):
        for phase in ("context_ready", "baseline_pending", "baseline_passed",
                      "native_pending", "complete", "failed"):
            with self.subTest(phase=phase):
                _, _, launches, _ = await F.OLD.launch_fixture(True, comparison_phase=phase)
                self.assertNotIn("main_world_eval", launches[0])
                self.assertNotIn("exclude_addons", launches[0])

    async def test_explicit_prepared_addon_is_preserved(self):
        addon = {"path": "fixture-prepared-addon", "excluded_default": object()}
        _, _, launches, _ = await F.OLD.launch_fixture(
            True, comparison_phase="context_pending", memory_addon=addon)
        self.assertEqual(launches[0]["addons"], [addon["path"]])
        self.assertEqual(launches[0]["exclude_addons"], [addon["excluded_default"]])
        self.assertIs(launches[0]["main_world_eval"], True)

    async def test_main_world_rpc_failure_never_retries_in_default_realm(self):
        value = self.fixture()
        await value.prepare_context()
        value.page.fail_sdk = True
        result = await value.baseline()
        self.assertFalse(result["accepted"])
        self.assertEqual(result["failure"], "preparation")
        self.assertEqual((value.mints, value.fetches), (1, 0))
        self.assertEqual(value.scope.comparison_phase, "failed")
        scripts = [script for script, _ in value.evaluations]
        self.assertEqual(scripts.count("mw:" + C.COMPARISON_MINT), 1)
        self.assertTrue(all(script.startswith("mw:") for script in scripts))
        self.assertNotIn("private-fixture-mint-error", json.dumps(result))
        with self.assertRaises(ValueError):
            await value.native()


if __name__ == "__main__":
    unittest.main()
