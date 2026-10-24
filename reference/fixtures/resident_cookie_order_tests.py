"""UNRUN inert resident cookie-order regressions; no browser or network."""
import unittest

import resident_bootstrap_tests as bootstrap_fixture
import resident_crossover_tests as crossover
from resident_activity import ResidentRefused
import resident_bootstrap as bootstrap


def cookie(name, value, path='/'):
    return dict(name=name, value=value, domain='.framer.com', path=path, secure=True)


class ResidentCrossoverCookieOrderTests(unittest.IsolatedAsyncioTestCase):
    asyncSetUp = crossover.ResidentCrossoverTests.asyncSetUp
    asyncTearDown = crossover.ResidentCrossoverTests.asyncTearDown
    phase = crossover.ResidentCrossoverTests.phase
    baseline = crossover.ResidentCrossoverTests.baseline

    async def test_full_pair_with_interleaved_cookie_rows(self):
        self.context.rows += [cookie('extra', 'first', '/ai'),
                              cookie('marker', 'stable'), cookie('extra', 'second')]
        rows = list(self.context.rows)
        def reorder():
            order = (3, 2, 0, 4, 1) if len(self.context.calls) % 2 else (2, 1, 4, 3, 0)
            self.context.rows = [rows[i] for i in order]
        self.context.hook = reorder
        acquire = await self.control.command(self.req)
        baseline = await self.control.command(self.baseline())
        self.assertTrue(baseline['accepted'])
        native = await self.control.command(self.phase('mint_native'))
        expected = ('marker=stable; extra=first; session=synthetic-session; '
                    'extra=second; __cuid=synthetic-device')
        for reply in (acquire, baseline, native):
            self.assertEqual(reply['cookie_header'], expected)
        self.assertEqual(self.page.tokens, ['synthetic-token-1', 'synthetic-token-2'])
        self.assertEqual(sum(kind == 'fetch' for kind, _ in self.page.calls), 1)
        receipt = await self.control.command(self.phase(
            'finish', native_state='settled', admission='resume'))
        self.assertEqual((receipt['state'], receipt['browser_settled']), ('released', True))
        self.assertTrue(self.page.pin.disposed)
        self.assertIsNone(self.life.activity.current)

    async def refuse_change(self, mutate):
        def after(kind):
            if kind == 'capture':
                mutate()
        self.page.after = after
        with self.assertRaisesRegex(ResidentRefused, '^resident authorization refused$'):
            await self.control.command(self.req)
        self.assertEqual(self.page.tokens, [])
        self.assertFalse(any(kind == 'fetch' for kind, _ in self.page.calls))
        self.assertTrue(self.life.activity.fenced)

    async def test_changed_cookie_value_refuses_before_mint(self):
        self.context.rows.append(cookie('extra', 'before'))
        await self.refuse_change(lambda: self.context.rows[-1].update(value='after'))

    async def test_changed_cookie_membership_refuses_before_mint(self):
        await self.refuse_change(lambda: self.context.rows.append(cookie('new', 'value')))

    async def test_duplicate_name_reordering_refuses_before_mint(self):
        self.context.rows += [cookie('extra', 'first', '/ai'), cookie('extra', 'second')]
        def swap():
            self.context.rows[-2:] = reversed(self.context.rows[-2:])
        await self.refuse_change(swap)

    async def test_invalid_cookie_syntax_refuses_before_mint(self):
        self.context.rows.append(cookie('extra', 'valid'))
        await self.refuse_change(lambda: self.context.rows[-1].update(value='bad;value'))


class ResidentBootstrapCookieOrderTests(unittest.IsolatedAsyncioTestCase):
    asyncSetUp = bootstrap_fixture.ResidentBootstrapTests.asyncSetUp
    asyncTearDown = bootstrap_fixture.ResidentBootstrapTests.asyncTearDown

    async def test_reordered_distinct_names_preserve_snapshot(self):
        self.context.rows.append(cookie('extra', 'stable'))
        self.page.hook = lambda: self.context.rows.reverse()
        result = await bootstrap.command(self.host, self.request)
        self.assertTrue(result['browser_settled'])
        self.assertEqual(self.page.calls, [bootstrap.READ_JS])
        self.assertEqual(len(self.context.calls), 2)
        self.assertIsNone(self.life.activity.current)

    async def test_duplicate_name_reordering_refuses(self):
        self.context.rows += [cookie('extra', 'first', '/ai'), cookie('extra', 'second')]
        self.page.hook = lambda: self.context.rows.reverse()
        with self.assertRaisesRegex(ResidentRefused, '^resident bootstrap refused$'):
            await bootstrap.command(self.host, self.request)
        self.assertEqual(self.page.calls, [bootstrap.READ_JS])
        self.assertEqual(len(self.context.calls), 2)
        self.assertIsNone(self.life.activity.current)


if __name__ == '__main__':
    unittest.main()
