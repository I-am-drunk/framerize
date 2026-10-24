"""Fixed diagnostic facts and resident ownership, with synthetic RPCs only."""
import contextlib
import asyncio
import io
import json
import unittest
from unittest.mock import patch

import resident_crossover_tests as v2
from resident_authorization import SDK_COUNTERS, _resident_sdk_evidence

LEASE = '55555555-5555-4555-8555-555555555555'


def evidence(**extra):
    return dict(complete=True, **dict.fromkeys(SDK_COUNTERS, 0)) | extra


class ResidentSdkFactsTests(unittest.TestCase):
    def emit(self, value, phase='baseline', lease=LEASE):
        out = io.StringIO()
        with contextlib.redirect_stderr(out):
            _resident_sdk_evidence(lease, phase, value)
        return [json.loads(line) for line in out.getvalue().splitlines()]

    def test_exact_fixed_counters_keep_incomplete_coverage(self):
        value = evidence(complete=False, v2=1, token_generation_timeout=1)
        self.assertEqual(self.emit(value), [dict(event='resident.mint.sdk',
            lease=LEASE, phase='baseline', available=True, **value)])

    def test_raw_or_malformed_fields_are_never_emitted(self):
        for value in (None, evidence(token='private'), evidence(v3=True),
                      evidence(v3=-1), evidence(v3=1001), evidence(complete=1)):
            self.assertEqual(self.emit(value), [dict(event='resident.mint.sdk',
                lease=LEASE, phase='baseline', available=False)])

    def test_invalid_identity_or_phase_emits_nothing(self):
        self.assertEqual(self.emit(evidence(), phase='private'), [])
        self.assertEqual(self.emit(evidence(), lease='private'), [])

    def test_broken_sink_does_not_propagate(self):
        with patch('resident_authorization.print', side_effect=OSError('private')):
            _resident_sdk_evidence(LEASE, 'native', evidence(v3=1))


class ResidentSdkWorkflowTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.h = v2.ResidentCrossoverTests()
        await self.h.asyncSetUp()
        self.out = io.StringIO()
        self.sink = contextlib.redirect_stderr(self.out)
        self.sink.__enter__()

    async def asyncTearDown(self):
        try:
            await self.h.asyncTearDown()
        finally:
            self.sink.__exit__(None, None, None)

    def observe(self, value, extra=None):
        original = self.h.page.pin.evaluate
        async def evaluate(source, args):
            reply = await original(source, args)
            if source == v2.RESIDENT_PINNED_MINT_JS:
                reply.update(sdk=value)
                reply.update(extra or {})
            return reply
        self.h.page.pin.evaluate = evaluate

    def events(self):
        return [json.loads(line) for line in self.out.getvalue().splitlines()]

    async def test_two_mints_keep_separate_events_outside_wire_replies(self):
        self.observe(evidence(v3=1))
        baseline = await self.h.passed()
        native = await self.h.control.command(self.h.phase('mint_native'))
        events = self.events()
        self.assertEqual([e['phase'] for e in events], ['baseline', 'native'])
        self.assertTrue(all(e['lease'] == self.h.req['lease'] and e['v3'] == 1
                            and e['complete'] for e in events))
        self.assertNotIn('sdk', baseline)
        self.assertNotIn('sdk', native)
        self.assertNotIn(native['token'], self.out.getvalue())
        self.assertEqual(len(self.h.page.tokens), 2)
        self.assertEqual(sum(k == 'fetch' for k, _ in self.h.page.calls), 1)

    async def test_unknown_diagnostic_cannot_refuse_a_valid_token(self):
        self.observe({'raw': 'private'})
        await self.h.passed()
        self.assertFalse(self.events()[0]['available'])
        self.assertNotIn('private', self.out.getvalue())

    async def test_counter_cannot_accept_unknown_wire_fields(self):
        self.observe(evidence(v3=1), {'unknown': 'private'})
        await self.h.acquired()
        reply = await self.h.control.command(self.h.baseline())
        self.assertFalse(reply['accepted'])
        self.assertTrue(self.h.life.activity.fenced)
        self.assertEqual(self.events()[0]['v3'], 1)
        self.assertFalse(any(k == 'fetch' for k, _ in self.h.page.calls))

    async def test_counter_cannot_accept_invalid_token(self):
        self.observe(evidence(v3=1), {'token': ''})
        await self.h.acquired()
        reply = await self.h.control.command(self.h.baseline())
        self.assertFalse(reply['accepted'])
        self.assertFalse(any(k == 'fetch' for k, _ in self.h.page.calls))

    async def test_cancelled_waiter_keeps_observation_with_raw_owner(self):
        self.observe(evidence(v3=1))
        await self.h.acquired()
        self.h.page.hold = 'mint'
        self.h.page.release.clear()
        waiter = asyncio.create_task(self.h.control.command(self.h.baseline()))
        await self.h.page.entered.wait()
        waiter.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await waiter
        self.assertEqual(self.events(), [])
        self.assertIsNotNone(self.h.life.activity.current)
        self.h.page.release.set()
        await self.h.asyncTearDown()
        self.assertEqual(len(self.events()), 1)
        self.assertEqual(self.events()[0]['v3'], 1)
        self.assertTrue(self.h.life.activity.fenced)
        self.assertFalse(any(k == 'fetch' for k, _ in self.h.page.calls))

    async def test_context_drift_retains_event_but_refuses_fetch(self):
        self.observe(evidence(v2=1))
        await self.h.acquired()
        def drift(kind):
            if kind == 'mint':
                self.h.page.url = 'https://framer.com/changed'
        self.h.page.after = drift
        reply = await self.h.control.command(self.h.baseline())
        self.assertFalse(reply['accepted'])
        self.assertEqual(self.events()[0]['v2'], 1)
        self.assertFalse(any(k == 'fetch' for k, _ in self.h.page.calls))


if __name__ == '__main__':
    unittest.main()
