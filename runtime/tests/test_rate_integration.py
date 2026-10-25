"""Exercise durable charging through the real host, with no real browser."""
import asyncio
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_host import app
from framerize_accounts.journal import RateJournal
from framerize_host.browser import driver
from test_lifecycle import FakeBrowser
from test_rates import Clock


class RateIntegrationTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.journal = RateJournal(directory.name)
        self.addCleanup(self.journal.close)
        override = patch.object(app, 'ROOT', directory.name)
        override.start()
        self.addCleanup(override.stop)
        protocol = patch.object(driver, 'verify_close_protocol', lambda: None)
        protocol.start()
        self.addCleanup(protocol.stop)
        self.host = app.Host(self.journal)
        self.host.rate_clock = self.clock = Clock()
        self.life = self.host._lifecycle('account')
        self.device = self.life.rates.ensure_devices(1)[0]
        self.acc = app.Account(None, None, None, self.life)
        self.releases, self.tasks = [], []

    async def asyncTearDown(self):
        for release in self.releases:
            release.set()
        await asyncio.gather(*self.tasks, return_exceptions=True)
        await asyncio.gather(*self.life.activity.pending, return_exceptions=True)
        await self.host.close()

    async def test_confirmed_close_saves_history_and_keeps_account_aliases(self):
        browser = FakeBrowser()
        self.life.browser, self.life.phase = browser, 'resident'
        self.life.close_receipt = app.BrowserCloseReceipt(browser)
        self.acc.browser = browser
        self.host.accounts['account'] = self.acc
        requests, cooling = self.acc.dev_reqs, self.acc.dev_cooling
        self.host._note_request(self.acc, self.device)
        self.host._cool_device(self.acc, self.device, 'account')
        self.assertIs(self.life.requests, requests)
        self.assertIs(self.life.cooling, cooling)
        self.assertEqual(list(requests[self.device]), [1000])
        self.assertTrue(await self.host._close_account('account'))
        saved = self.journal.load('account')['state']
        self.assertTrue(saved['clean'])
        self.assertEqual(saved['requests'], [[self.device, [1000]]])
        self.assertEqual(saved['cooling'], [[self.device, 1600]])
        self.assertEqual(saved['primary'], self.device)
        self.assertFalse(browser.connected)

    async def test_cancelled_mint_keeps_only_its_own_rpc_and_pacing_owned(self):
        entered, release = asyncio.Event(), asyncio.Event()
        foreign_entered, foreign_release = asyncio.Event(), asyncio.Event()
        self.releases.extend((release, foreign_release))
        effects = []
        async def port(started, stop):
            started.set()
            await stop.wait()
        foreign = asyncio.create_task(self.host._rpc(
            'account', port, foreign_entered, foreign_release))
        self.tasks.append(foreign)
        await asyncio.wait_for(foreign_entered.wait(), 1)
        async def mint():
            async with self.host._mint_gate(self.acc):
                await self.host._rpc('account', port, entered, release)
                effects.append('unwanted followup')
        task = asyncio.create_task(mint())
        self.tasks.append(task)
        await asyncio.wait_for(entered.wait(), 1)
        self.assertTrue(self.journal.load('account')['state']['pending_mint'])
        task.cancel()
        await asyncio.sleep(0)
        self.assertTrue(self.acc.mint_lock.locked())
        self.clock.stamp = 1200
        release.set()
        result = await asyncio.wait_for(asyncio.gather(task, return_exceptions=True), 1)
        self.assertIsInstance(result[0], asyncio.CancelledError)
        self.assertEqual(effects, [])
        self.assertFalse(foreign.done())
        self.assertFalse(self.acc.mint_lock.locked())
        saved = self.journal.load('account')['state']
        self.assertFalse(saved['pending_mint'])
        self.assertEqual(saved['last_mint'], 1200)

    async def test_failed_mint_commit_never_enters_external_work(self):
        effects = []
        with patch.object(self.journal, 'save', side_effect=OSError('storage unavailable')):
            with self.assertRaises(OSError):
                async with self.host._mint_gate(self.acc):
                    effects.append('external work')
        self.assertEqual(effects, [])
        self.assertFalse(self.acc.mint_lock.locked())
        self.assertFalse(self.journal.load('account')['state']['pending_mint'])
        with self.assertRaisesRegex(RuntimeError, 'recovery'):
            self.host._note_request(self.acc, self.device)
