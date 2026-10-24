"""UNRUN: resident replies must retain the existing safe refusal classification.

These regressions require a nullable `refusal` object on the baseline reply.
They intentionally fail against 61afa7e, which drops that already-computed fact.
The owner must update the strict Rust decoder with the Python producer.
Only the existing synthetic crossover harness runs; no browser or network.
"""
import json
import unittest

import resident_crossover_tests as pair
from resident_activity import ResidentRefused

MARKER = 'synthetic-private-response-marker'
SECURITY = {'error': {
    'status': 403,
    'message': 'This request was blocked by our security checks',
    'data': {'reason': 'request_blocked', 'private': MARKER},
    'ref': MARKER, 'timestamp': '2000-01-01T00:00:00Z'}}
CASTLE = {'policy': 'castle', 'basis': 'framer_security_envelope'}


class ResidentCrossoverRefusalTests(unittest.IsolatedAsyncioTestCase):
    asyncSetUp = pair.ResidentCrossoverTests.asyncSetUp
    asyncTearDown = pair.ResidentCrossoverTests.asyncTearDown
    phase = pair.ResidentCrossoverTests.phase
    baseline = pair.ResidentCrossoverTests.baseline
    acquired = pair.ResidentCrossoverTests.acquired

    async def refused(self, raw, after=None):
        await self.acquired()
        self.page.raw = raw
        if after is not None:
            self.page.after = after
        reply = await self.control.command(self.baseline())
        self.assertFalse(reply['accepted'])
        self.assertEqual(reply['status'], raw['status'])
        before = list(self.page.calls)
        with self.assertRaises(ResidentRefused):
            await self.control.command(self.phase('mint_native'))
        self.assertEqual(self.page.calls, before)
        self.assertEqual(len(self.page.tokens), 1)
        self.assertEqual(sum(k == 'fetch' for k, _ in self.page.calls), 1)
        self.assertNotIn('body', reply)
        self.assertNotIn('token', reply)
        self.assertNotIn(MARKER, json.dumps(reply))
        return reply

    async def test_security_envelope_keeps_only_fixed_refusal_fields(self):
        reply = await self.refused(pair.response(SECURITY, status=403, elapsed_ms=2000))
        self.assertTrue(reply['body_complete'] and reply['json_object'])
        self.assertTrue(reply['browser_settled'])
        self.assertEqual(reply['refusal'], CASTLE)

    async def test_provider_marker_stays_upstream_despite_fast_response(self):
        value = {'error': {'message': 'provider refused', 'private': MARKER}}
        reply = await self.refused(pair.response(value, status=403, elapsed_ms=1))
        self.assertTrue(reply['body_complete'] and reply['browser_settled'])
        self.assertEqual(reply['refusal'],
                         {'policy': 'upstream', 'basis': 'provider_marker'})

    async def test_page_drift_keeps_security_refusal_and_native_fence(self):
        def navigate(kind):
            if kind == 'fetch':
                self.page.url = pair.v1.URL + '?changed'
        reply = await self.refused(
            pair.response(SECURITY, status=403, elapsed_ms=2000), navigate)
        self.assertTrue(reply['body_complete'] and reply['json_object'])
        self.assertTrue(reply['browser_settled'])
        self.assertTrue(self.life.activity.fenced)
        self.assertEqual(reply['refusal'], CASTLE)

    async def test_incomplete_403_stays_unclassified_and_native_ineligible(self):
        reply = await self.refused(pair.response(
            SECURITY, status=403, complete=False, failure='body', elapsed_ms=2000))
        self.assertFalse(reply['body_complete'] or reply['json_object'])
        self.assertTrue(reply['browser_settled'])
        self.assertIsNone(reply['refusal'])

    async def test_foreign_refusal_and_private_fields_do_not_create_evidence(self):
        reply = await self.refused(pair.response(
            {'detail': MARKER}, status=500, refusal=CASTLE, token=MARKER,
            error=MARKER, elapsed_ms=2000))
        self.assertTrue(reply['body_complete'] and reply['browser_settled'])
        self.assertIsNone(reply['refusal'])


if __name__ == '__main__':
    unittest.main()
