"""Readiness validates durable state without opening a browser or sending traffic."""
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_accounts.journal import RateJournal
from framerize_accounts.rates import AccountRates, RatePolicy
from framerize_host import app
from test_rates import Clock


class HistoryReadinessTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        override = patch.object(app, 'ROOT', temporary.name)
        override.start()
        self.addCleanup(override.stop)
        self.journal = RateJournal(temporary.name)
        self.addCleanup(self.journal.close)
        self.host = app.Host(self.journal)
        self.host.rate_clock = self.clock = Clock()

    async def asyncTearDown(self):
        await self.host.close()

    async def test_returned_browser_accepts_history_with_reserves_now_disabled(self):
        rates = AccountRates(self.journal, 'account', clock=self.clock)
        rates.ensure_devices(1)
        rates.allocate_reserve()
        rates.confirm_runtime_stopped()
        self.host.rate_policy = RatePolicy(reserves=0)
        reply = await self.host.handle(dict(cmd='history_status', accounts=['account']))
        self.assertEqual(reply, dict(ready=True, accounts=1))
        self.assertEqual(self.host.accounts, {})
        self.assertIsNone(self.host._browser_runtime)
        self.assertIsNone(self.host._playwright)
        self.assertEqual(self.host.lifecycles['account'].rates.state['reserve_used'], 1)

    async def test_bad_persisted_account_prevents_readiness_before_browser_work(self):
        self.journal.save('account', {'invalid': 'private-fixture'})
        with self.assertRaisesRegex(ValueError, 'incompatible account rate history'):
            await self.host.handle(dict(cmd='history_status', accounts=['account']))
        self.assertEqual(self.host.accounts, {})
        self.assertIsNone(self.host._browser_runtime)

    async def test_unknown_cleanup_remains_unready(self):
        rates = AccountRates(self.journal, 'account', clock=self.clock)
        rates.begin_mint()
        with self.assertRaisesRegex(RuntimeError, 'requires recovery'):
            await self.host.handle(dict(cmd='history_status', accounts=['account']))
        self.assertTrue(self.journal.load('account')['state']['pending_mint'])
