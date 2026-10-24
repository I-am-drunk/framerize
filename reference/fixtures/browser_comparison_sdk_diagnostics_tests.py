"""Inert logger-window privacy checks. No SDK, browser or network."""
import importlib.util
import json
from pathlib import Path
from types import SimpleNamespace
import unittest

SPEC = importlib.util.spec_from_file_location(
    '_sdk_diagnostics', Path(__file__).with_name('browser_comparison_mint_diagnostics_tests.py'))
M = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(M)
C, D, F = M.C, M.D, M.F


def counters(**values):
    return dict.fromkeys(C.SDK_COUNTERS, 0) | {'complete': True} | values


class SDKCounterTests(unittest.TestCase):
    def capture(self, value):
        events = []
        C.comparison_sdk_evidence(SimpleNamespace(comparison_sdk_emit=events.append), value)
        self.assertEqual(len(events), 1)
        self.assertNotIn(M.SECRET, json.dumps(events))
        return events[0]

    def test_exact_counters_preserve_incomplete_coverage(self):
        value = counters(v3=1, v2=2, complete=False, sdk_error=1000)
        self.assertEqual(self.capture(value), {'available': True, **value})

    def test_malformed_or_private_metadata_is_wholly_unknown(self):
        values = [None, [], M.SECRET, {}, counters(secret=M.SECRET),
                  {key: value for key, value in counters().items() if key != 'v2'}]
        values += [counters(v3=value) for value in (-1, 1001, True, 1.0, M.SECRET, None)]
        values += [counters(complete=value) for value in (0, 1, None, M.SECRET)]
        for value in values:
            with self.subTest(value_type=type(value).__name__):
                self.assertEqual(self.capture(value), {'available': False})

    def test_sink_failure_is_nonfatal_and_observations_are_capped_across_phases(self):
        scope, events = SimpleNamespace(), []
        def sink(event, **facts):
            events.append(dict(event=event, **facts))
        for purpose in ('baseline', 'native_token', 'native_token'):
            request = {'purpose': purpose}
            if purpose == 'baseline': request = {'cmd': 'browser_baseline_once'}
            C.comparison_attach_progress(scope, request, sink)
            C.comparison_sdk_evidence(scope, counters(v3=1))
        self.assertEqual(len(events), 2)
        self.assertEqual([e['mint'] for e in events], [1, 2])
        self.assertEqual([e['purpose'] for e in events], ['baseline', 'native_token'])
        self.assertEqual(events[0]['trace'], events[1]['trace'])
        def broken(_): raise RuntimeError(M.SECRET)
        C.comparison_sdk_evidence(SimpleNamespace(comparison_sdk_emit=broken), counters())


class SDKWorkflowTests(unittest.IsolatedAsyncioTestCase):
    asyncSetUp = F.BrowserComparisonTests.asyncSetUp
    asyncTearDown = F.BrowserComparisonTests.asyncTearDown
    fixture = D.ComparisonDiagnosticWorkflowTests.fixture
    mint_reply = M.ComparisonMintDiagnosticsTests.mint_reply
    success_reply = M.ComparisonMintDiagnosticsTests.success_reply

    async def test_pair_keeps_two_mints_one_fetch_and_separate_safe_events(self):
        value = self.fixture()
        await value.prepare_context()
        evaluate = value.page.evaluate
        async def observed(script, args=None):
            result = await evaluate(script, args)
            if script == 'mw:' + C.COMPARISON_MINT:
                return {**result, 'sdk': counters(v3=1)}
            return result
        value.page.evaluate = observed
        self.assertTrue((await value.baseline())['accepted'])
        native = await value.native()
        self.assertNotIn('sdk', native)
        self.assertEqual((value.mints, value.fetches), (2, 1))
        self.assertEqual([e['mint'] for e in value.sdk_events], [1, 2])
        self.assertTrue(all(e['available'] and e['v3'] == 1 for e in value.sdk_events))
        self.assertNotIn(native['token'], json.dumps(value.sdk_events))

    async def test_unknown_metadata_cannot_refuse_a_valid_token(self):
        value = self.fixture()
        await value.prepare_context()
        reply = self.success_reply(value, M.SECRET) | {'sdk': {'private': M.SECRET}}
        self.mint_reply(value, reply)
        self.assertTrue((await value.baseline())['accepted'])
        self.assertEqual(value.sdk_events[-1]['available'], False)
        self.assertNotIn(M.SECRET, json.dumps(value.sdk_events))
        self.assertEqual((value.mints, value.fetches), (1, 1))

    async def test_v3_counts_cannot_accept_a_refused_or_invalid_mint(self):
        for body, stage in (({'failure_stage': 'post_device'}, 'mint_post_device'),
                            ({'token': ''}, 'mint_invalid_token')):
            value = self.fixture()
            await value.prepare_context()
            reply = (body if 'failure_stage' in body else self.success_reply(value, ''))
            self.mint_reply(value, reply | {'sdk': counters(v3=1)})
            result = await value.baseline()
            self.assertFalse(result['accepted'])
            self.assertEqual(value.events[-1]['stage'], stage)
            self.assertEqual((value.mints, value.fetches), (1, 0))
            self.assertEqual(value.sdk_events[-1]['v3'], 1)


if __name__ == '__main__':
    unittest.main()
