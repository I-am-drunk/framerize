"""UNRUN: real cooldown side effects with synthetic resident/browser ports.

The page-drift case checks an owner-checked post-fetch history update.
No SDK, network, private state, installed host or browser is used.
"""
import ast
import time
import unittest
from types import MethodType, SimpleNamespace

import resident_crossover_tests as pair
from resident_activity import ResidentRefused


def host_cooler():
    path = pair.v1.ROOT / 'scripts/camoufox_host.py'
    tree = ast.parse(path.read_text(), filename=str(path))
    host = next(n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == 'Host')
    method = next(n for n in host.body if isinstance(n, ast.FunctionDef)
                  and n.name == '_cool_device')
    value = next(n.value for n in tree.body if isinstance(n, ast.Assign)
                 and any(isinstance(t, ast.Name) and t.id == 'DEV_COOLDOWN_S'
                         for t in n.targets))
    seconds = ast.literal_eval(value)
    scope = {'time': time, 'DEV_COOLDOWN_S': seconds, 'trace': lambda _: None}
    exec(compile(ast.Module(body=[method], type_ignores=[]), str(path), 'exec'), scope)
    return scope['_cool_device'], seconds


class ResidentCooldownEffectsTests(unittest.IsolatedAsyncioTestCase):
    phase = pair.ResidentCrossoverTests.phase
    baseline = pair.ResidentCrossoverTests.baseline
    acquired = pair.ResidentCrossoverTests.acquired
    asyncTearDown = pair.ResidentCrossoverTests.asyncTearDown

    async def asyncSetUp(self):
        await pair.ResidentCrossoverTests.asyncSetUp(self)
        cooler, self.cooldown_seconds = host_cooler()
        self.host._cool_device = MethodType(cooler, self.host)
        self.pool = {'ambient': {'dev': None}, 'other': {'dev': 'synthetic-other'}}
        self.acc.token_pool = self.pool
        self.original_cooling = self.acc.dev_cooling

    async def refused(self, after=None):
        await self.acquired()
        self.page.raw = pair.response({'error': 'refused'}, status=403, elapsed_ms=1)
        if after is not None:
            self.page.after = after
        self.started = time.monotonic()
        reply = await self.control.command(self.baseline())
        self.assertFalse(reply['accepted'])
        self.assertEqual(reply['status'], 403)
        self.assertTrue(reply['body_complete'] and reply['browser_settled'])
        before = list(self.page.calls)
        with self.assertRaises(ResidentRefused):
            await self.control.command(self.phase('mint_native'))
        self.assertEqual(self.page.calls, before)
        self.assertEqual(len(self.page.tokens), 1)
        self.assertEqual(sum(k == 'fetch' for k, _ in self.page.calls), 1)

    def assert_original_cooled(self):
        self.assertIn(None, self.original_cooling)
        self.assertGreaterEqual(self.original_cooling[None], self.started + self.cooldown_seconds)
        self.assertEqual(self.pool, {'other': {'dev': 'synthetic-other'}})

    async def test_current_context_cools_and_removes_only_matching_queued_tokens(self):
        await self.refused()
        self.assert_original_cooled()

    async def test_page_drift_preserves_original_cooldown_and_token_eviction(self):
        from resident_crossover_provenance_tests import Events
        evidence = self.acc.completed_browser_http200
        events = [Events(obj) for obj in (self.page, self.context, self.browser)]
        self.assertTrue(evidence.watch())
        def navigate(kind):
            if kind == 'fetch':
                events[0].fire('framenavigated', self.page.main_frame)
        await self.refused(navigate)
        self.assertEqual(self.page.url, pair.v1.URL)
        self.assertIsNone(self.acc.completed_browser_http200)
        self.assertTrue(self.life.activity.fenced)
        self.assert_original_cooled()

    async def test_replaced_account_receives_no_stale_cooldown_or_token_eviction(self):
        replacement = SimpleNamespace(lifecycle=self.life, dev_cooling={},
            token_pool={'replacement': {'dev': None}})
        def replace(kind):
            if kind == 'fetch':
                self.host.accounts[pair.v1.ACCOUNT] = replacement
        await self.refused(replace)
        self.assertEqual(self.original_cooling, {})
        self.assertEqual(replacement.dev_cooling, {})
        self.assertEqual(replacement.token_pool, {'replacement': {'dev': None}})
        self.assertEqual(self.pool, {'ambient': {'dev': None},
                                    'other': {'dev': 'synthetic-other'}})

    async def test_replaced_history_map_keeps_both_maps_and_token_pool_untouched(self):
        replacement = {}
        def replace(kind):
            if kind == 'fetch':
                self.page.url = pair.v1.URL + '?changed'
                self.acc.dev_cooling = self.life.cooling = replacement
        await self.refused(replace)
        self.assertTrue(self.life.activity.fenced)
        self.assertEqual(self.original_cooling, {})
        self.assertEqual(replacement, {})
        self.assertEqual(self.pool, {'ambient': {'dev': None},
                                    'other': {'dev': 'synthetic-other'}})


    async def test_replaced_token_pool_preserves_both_pools_and_history(self):
        replacement = {'replacement': {'dev': None}}
        def replace(kind):
            if kind == 'fetch':
                self.page.url = pair.v1.URL + '?changed'
                self.acc.token_pool = replacement
        await self.refused(replace)
        self.assertEqual(self.original_cooling, {})
        self.assertEqual(replacement, {'replacement': {'dev': None}})
        self.assertEqual(self.pool, {'ambient': {'dev': None},
                                    'other': {'dev': 'synthetic-other'}})


if __name__ == '__main__':
    unittest.main()
