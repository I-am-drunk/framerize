"""Admission changes during token waiting must not become upstream requests."""
import asyncio
from types import SimpleNamespace
import tempfile
import unittest
from unittest.mock import AsyncMock, patch

import test_post as post
import test_rates as rate_tests
from framerize_accounts.journal import RateJournal
from framerize_accounts.rates import AccountRates, RatePolicy


class LocalCapacityTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        post.PostTests.setUp(self)
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.journal = RateJournal(directory.name)
        self.addCleanup(self.journal.close)
        self.clock = rate_tests.Clock()
        self.rates = AccountRates(self.journal, "fixture",
                                  RatePolicy(capacity=2, reserves=0), self.clock)
        self.devices = self.rates.ensure_devices(2)
        self.acc.lifecycle = SimpleNamespace(rates=self.rates)
        self.acc.devices = list(self.devices)
        del self.host._pick_device_sticky
        del self.host._note_request
        self.host._run_job = AsyncMock(return_value={
            "status": 200, "usage": None, "body": "{}"})
        self.request = dict(cmd="ai_post", account="fixture", path="/fixture",
                            body={"request": {"model": "fixture"}}, defer_retries=True)
        self.pending = []

    async def asyncTearDown(self):
        for task in self.pending:
            if not task.done():
                task.cancel()
        await asyncio.gather(*self.pending, return_exceptions=True)

    async def begin(self, ambient=False):
        selected, resume = asyncio.Event(), asyncio.Event()
        async def acquire(acc, page, account, dev):
            self.assertEqual(dev, self.devices[0])
            selected.set()
            await resume.wait()
            return "fixture-slot", "fixture-token", None if ambient else dev
        self.host._token_acquire = AsyncMock(side_effect=acquire)
        pending = asyncio.create_task(self.host._handle(self.request))
        self.pending.append(pending)
        await asyncio.wait_for(selected.wait(), 1)
        return pending, resume

    async def check_race(self, *, cooldown=False, ambient=False, all_busy=False):
        pending, resume = await self.begin(ambient)
        device = None if ambient else self.devices[0]
        if cooldown:
            self.rates.cool_device(device)
        else:
            for _ in range(self.rates.policy.capacity):
                self.rates.charge_request(device)
        if all_busy:
            self.rates.cool_device(self.devices[1])
        before = self.journal.load("fixture")
        resume.set()
        result = await asyncio.wait_for(pending, 1)
        wait = 600 if ambient or all_busy else 0
        self.assertEqual(result["status"], 0)
        self.assertIsNone(result["usage"])
        self.assertRegex(result["body"], rf"^velocity-paused: .*retry in {wait}s$")
        self.assertEqual(self.journal.load("fixture"), before)
        self.host._run_job.assert_not_awaited()
        self.host._token_release.assert_called_once_with(self.acc, "fixture-slot")
        self.host._cool_device.assert_not_called()
        event = self.events.call_args.kwargs
        self.assertEqual((event["status"], event["outcome"]), (0, "error"))
        if wait:
            self.clock.stamp += wait + 1
        expected = device if wait else self.devices[1]
        async def acquire(acc, page, account, dev):
            used = None if ambient else dev
            self.assertEqual(used, expected)
            return "next-slot", "fixture-token", used
        self.host._token_acquire = AsyncMock(side_effect=acquire)
        retried = await asyncio.wait_for(self.host._handle(self.request), 1)
        self.assertEqual(retried["status"], 200)
        self.host._run_job.assert_awaited_once()
        self.host._token_acquire.assert_awaited_once()
        self.assertEqual(self.host._token_release.call_count, 2)
        self.assertEqual(list(self.rates.state["requests"][expected]), [self.clock.stamp])

    async def test_budget_consumed_during_token_wait_allows_a_later_retry(self):
        await self.check_race()

    async def test_device_cooled_during_token_wait_allows_a_later_retry(self):
        await self.check_race(cooldown=True)

    async def test_all_saved_devices_busy_report_the_earliest_recovery(self):
        await self.check_race(all_busy=True)

    async def test_ambient_budget_does_not_use_an_unrelated_saved_device_delay(self):
        await self.check_race(ambient=True)

    async def test_saved_device_wait_uses_earliest_fractional_recovery(self):
        self.rates.cool_device(self.devices[1])
        self.clock.stamp += 400.25
        pending, resume = await self.begin()
        for _ in range(self.rates.policy.capacity):
            self.rates.charge_request(self.devices[0])
        before = self.journal.load("fixture")
        resume.set()
        result = await asyncio.wait_for(pending, 1)
        self.assertRegex(result["body"], r"; retry in 199s$")
        self.assertEqual(result["status"], 0)
        self.assertEqual(self.journal.load("fixture"), before)
        self.host._run_job.assert_not_awaited()
        self.host._token_release.assert_called_once_with(self.acc, "fixture-slot")

    async def test_failed_journal_write_still_fails_closed_and_releases_token(self):
        pending, resume = await self.begin()
        before = self.journal.load("fixture")
        with patch.object(self.journal, "save", side_effect=OSError("fixture write failed")):
            resume.set()
            with self.assertRaisesRegex(OSError, "^fixture write failed$"):
                await asyncio.wait_for(pending, 1)
        self.assertTrue(self.rates.failed)
        self.assertEqual(self.journal.load("fixture"), before)
        self.host._run_job.assert_not_awaited()
        self.host._token_release.assert_called_once_with(self.acc, "fixture-slot")
        with self.assertRaisesRegex(RuntimeError, "history requires recovery"):
            self.rates.charge_request(self.devices[1])

    async def test_unconfirmed_history_is_not_converted_to_a_capacity_wait(self):
        pending, resume = await self.begin()
        before = self.journal.load("fixture")
        self.rates.needs_cleanup = True
        resume.set()
        with self.assertRaisesRegex(RuntimeError, "history requires recovery"):
            await asyncio.wait_for(pending, 1)
        self.assertEqual(self.journal.load("fixture"), before)
        self.host._run_job.assert_not_awaited()
        self.host._token_release.assert_called_once_with(self.acc, "fixture-slot")

    async def test_cancellation_during_token_wait_never_charges_or_posts(self):
        pending, _resume = await self.begin()
        before = self.journal.load("fixture")
        pending.cancel("fixture cancel during token wait")
        with self.assertRaises(asyncio.CancelledError) as caught:
            await asyncio.wait_for(pending, 1)
        self.assertEqual(caught.exception.args, ("fixture cancel during token wait",))
        self.assertEqual(self.journal.load("fixture"), before)
        self.host._run_job.assert_not_awaited()
        self.host._token_release.assert_not_called()
        self.host._cool_device.assert_not_called()
