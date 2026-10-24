"""UNRUN inert integration cases: real provenance listeners, fake browser RPCs."""
import unittest

import resident_crossover_tests as pair
from resident_activity import ResidentRefused


class Events:
    def __init__(self, source):
        self.listeners = {}
        source.on, source.remove_listener = self.on, self.remove

    def on(self, event, callback):
        self.listeners.setdefault(event, []).append(callback)

    def remove(self, event, callback):
        self.listeners[event].remove(callback)

    def fire(self, event, *args):
        for callback in tuple(self.listeners.get(event, ())):
            callback(*args)


class ResidentCrossoverProvenanceTests(unittest.IsolatedAsyncioTestCase):
    phase = pair.ResidentCrossoverTests.phase
    baseline = pair.ResidentCrossoverTests.baseline
    acquired = pair.ResidentCrossoverTests.acquired
    passed = pair.ResidentCrossoverTests.passed

    async def asyncSetUp(self):
        await pair.ResidentCrossoverTests.asyncSetUp(self)
        self.events = [Events(obj) for obj in (self.page, self.context, self.browser)]
        self.evidence = self.acc.completed_browser_http200
        self.assertTrue(self.evidence.watch())

    async def asyncTearDown(self):
        try:
            await pair.ResidentCrossoverTests.asyncTearDown(self)
        finally:
            self.evidence.invalidate()
        self.assert_detached()

    def assert_detached(self):
        for source in self.events:
            self.assertFalse(any(source.listeners.values()))

    def navigate(self, frame=None):
        self.events[0].fire('framenavigated', self.page.main_frame if frame is None else frame)
        self.assertEqual(self.page.url, pair.v1.URL)

    async def test_same_url_navigation_before_acquire_refuses_without_rpc(self):
        self.navigate()
        self.assertIsNone(self.acc.completed_browser_http200)
        with self.assertRaises(ResidentRefused):
            await self.acquired()
        self.assertEqual(self.page.calls, [])
        self.assertEqual(self.context.calls, [])
        self.assertIsNone(self.life.activity.current)
        self.assert_detached()

    async def test_navigation_during_baseline_refuses_native_handoff(self):
        await self.acquired()
        self.page.after = lambda kind: self.navigate() if kind == 'fetch' else None
        reply = await self.control.command(self.baseline())
        self.assertFalse(reply['accepted'])
        self.assertEqual(reply['status'], 200)
        self.assertTrue(reply['body_complete'])
        self.assertTrue(reply['json_object'])
        self.assertTrue(reply['browser_settled'])
        self.assertIsNone(self.acc.completed_browser_http200)
        self.assertEqual(len(self.page.tokens), 1)
        self.assertEqual(sum(kind == 'fetch' for kind, _ in self.page.calls), 1)
        before = list(self.page.calls)
        with self.assertRaises(ResidentRefused):
            await self.control.command(self.phase('mint_native'))
        self.assertEqual(self.page.calls, before)
        self.assertTrue(self.life.activity.fenced)
        self.assert_detached()

    async def test_navigation_after_baseline_prevents_second_mint(self):
        await self.passed()
        self.navigate()
        before = list(self.page.calls)
        with self.assertRaises(ResidentRefused):
            await self.control.command(self.phase('mint_native'))
        self.assertEqual(self.page.calls, before)
        self.assertEqual(len(self.page.tokens), 1)
        self.assertTrue(self.life.activity.fenced)
        self.assert_detached()

    async def test_subframe_navigation_keeps_the_owned_pair_eligible(self):
        await self.acquired()
        self.page.after = lambda kind: self.navigate(object()) if kind == 'fetch' else None
        reply = await self.control.command(self.baseline())
        self.assertTrue(reply['accepted'])
        native = await self.control.command(self.phase('mint_native'))
        self.assertEqual(native['token'], 'synthetic-token-2')
        self.assertEqual(len(self.page.tokens), 2)
        self.assertEqual(sum(kind == 'fetch' for kind, _ in self.page.calls), 1)
        self.assertIs(self.acc.completed_browser_http200, self.evidence)
        reply = await self.control.command(self.phase(
            'finish', native_state='settled', admission='resume'))
        self.assertEqual(reply['state'], 'released')
        self.assertTrue(reply['browser_settled'])
        self.assertTrue(self.page.pin.disposed)


    async def test_navigation_keeps_settled_baseline_403_visible(self):
        # A known response survives drift without permitting a native mint.
        await self.acquired()
        self.page.raw = pair.response({'error': {
            'status': 403,
            'message': 'This request was blocked by our security checks',
            'data': {'reason': 'request_blocked'},
            'ref': 'synthetic-private-reference', 'timestamp': '2000-01-01T00:00:00Z'}},
            status=403, elapsed_ms=5000)
        self.page.after = lambda kind: self.navigate() if kind == 'fetch' else None
        reply = await self.control.command(self.baseline())
        self.assertFalse(reply['accepted'])
        self.assertTrue(reply['browser_settled'])
        self.assertTrue(reply['body_complete'])
        self.assertTrue(reply['json_object'])
        with self.assertRaises(ResidentRefused):
            await self.control.command(self.phase('mint_native'))
        self.assertEqual(len(self.page.tokens), 1)
        self.assertEqual(reply['status'], 403)
        self.assertEqual(reply['refusal'],
                         {'policy': 'castle', 'basis': 'framer_security_envelope'})
        self.assertNotIn('synthetic-private-reference', repr(reply))


if __name__ == '__main__':
    unittest.main()
