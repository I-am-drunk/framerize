"""Native ownership against temporary journals, without browser or SDK calls."""
import asyncio
from dataclasses import replace
from pathlib import Path
import sys
import tempfile
import unittest
import uuid
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_accounts.journal import RateJournal
from framerize_accounts.rates import AccountRates, RatePolicy
from framerize_native.owner import LedgerOwner, OwnershipError


class Clock:
    boot_id, stamp = 'fixture-boot', 1000.0

    def now(self):
        return self.stamp

    def day(self):
        return '2026-10-25'


class OwnerTests(unittest.IsolatedAsyncioTestCase):
    async def test_stale_copied_and_mismatched_grants_cannot_mutate_history(self):
        grant = self.open()
        before = self.owner._journal.load(self.account)
        for foreign in (replace(grant), replace(grant, account=self.other),
                        replace(grant, generation='stale'), grant.wire()):
            with self.subTest(grant=type(foreign).__name__), self.assertRaises(OwnershipError):
                await self.owner.admit_mint(foreign, 1000)
        with self.assertRaisesRegex(OwnershipError, 'conflict'):
            self.owner.open(self.account, grant.lease)
        self.assertEqual(self.owner._journal.load(self.account), before)
        sequence = await self.owner.admit_mint(grant, 1000)
        with self.assertRaisesRegex(OwnershipError, 'identity'):
            self.owner.finish_mint(grant, True, True)
        with self.assertRaisesRegex(OwnershipError, 'evidence'):
            self.owner.finish_mint(grant, sequence, 1)
        self.owner.finish_mint(grant, sequence, True)
        self.owner.close_grant(grant, True)
        with self.assertRaises(OwnershipError):
            self.owner.close_grant(grant, True)

    async def test_full_window_refuses_instead_of_selecting_another_device(self):
        grant = self.open()
        for _ in range(self.policy.capacity):
            sequence = await self.owner.admit_mint(grant, 1000)
            self.owner.finish_mint(grant, sequence, True)
            self.clock.stamp += 1
        with self.assertRaisesRegex(RuntimeError, 'budget'):
            await self.owner.admit_mint(grant, 1000)
        self.owner.close_grant(grant, True)
        with self.assertRaisesRegex(OwnershipError, 'unavailable'):
            self.open()
        self.assertEqual(self.owner._journal.load(self.account)['state']['primary'], grant.device)

    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root, self.clock = Path(directory.name), Clock()
        self.policy = RatePolicy(capacity=3, reserves=1)
        self.account, self.other = str(uuid.uuid4()), str(uuid.uuid4())
        with RateJournal(self.root) as journal:
            for account in (self.account, self.other):
                rates = AccountRates(journal, account, self.policy, self.clock)
                rates.ensure_devices(1)
                rates.allocate_reserve()
                rates.confirm_runtime_stopped()
            self.before = journal.load(self.account)['state']
        self.owner = self.reopen()

    def reopen(self):
        owner = LedgerOwner(self.root, self.policy, self.clock)
        self.addCleanup(owner.shutdown)
        return owner

    def test_only_existing_history_can_be_owned_and_writer_is_exclusive(self):
        with tempfile.TemporaryDirectory() as missing:
            with self.assertRaisesRegex(OwnershipError, 'existing rate history'):
                LedgerOwner(missing, self.policy, self.clock)
            self.assertEqual(list(Path(missing).iterdir()), [])
        with self.assertRaises(BlockingIOError):
            RateJournal(self.root)
        with self.assertRaises(BlockingIOError):
            self.reopen()
        unknown = str(uuid.uuid4())
        with self.assertRaisesRegex(OwnershipError, 'existing account history'):
            self.open(unknown)
        self.assertIsNone(self.owner._journal.load(unknown))
        self.assertTrue(self.owner.shutdown())
        with RateJournal(self.root) as journal:
            self.assertEqual(journal.load(self.account)['state'], self.before)

    async def test_shutdown_revokes_waiting_admission_without_clearing_history(self):
        first, second = self.open(), self.open()
        await self.owner.admit_mint(first, 1000)
        waiting = asyncio.create_task(self.owner.admit_mint(second, 10))
        await asyncio.sleep(0)
        self.assertFalse(self.owner.shutdown())
        with self.assertRaises(TimeoutError):
            await waiting
        with self.assertRaisesRegex(OwnershipError, 'closed'):
            self.open()
        self.owner = self.reopen()
        with self.assertRaisesRegex(RuntimeError, 'recovery'):
            self.open()
        state = self.owner._journal.load(self.account)['state']
        self.assertTrue(state['pending_mint'])
        self.assertEqual(state['requests'], [[first.device, [1000.0]]])

    def open(self, account=None):
        return self.owner.open(account or self.account, str(uuid.uuid4()))

    async def test_charges_and_saved_device_survive_confirmed_close(self):
        grant = self.open()
        self.assertEqual(grant.device, self.before['primary'])
        sequence = await self.owner.admit_mint(grant, 1000)
        state = self.owner._journal.load(self.account)['state']
        self.assertEqual(state['requests'], [[grant.device, [1000.0]]])
        self.assertTrue(state['pending_mint'])
        self.assertFalse(state['clean'])
        self.clock.stamp += 2
        self.assertTrue(self.owner.finish_mint(grant, sequence, True))
        self.assertTrue(self.owner.close_grant(grant, True))
        self.assertTrue(self.owner.shutdown())
        self.owner = self.reopen()
        after = self.owner._journal.load(self.account)['state']
        for field in ('devices', 'reserve', 'reserve_used', 'primary'):
            self.assertEqual(after[field], self.before[field])
        self.assertEqual(after['requests'], state['requests'])
        self.assertEqual(after['last_mint'], 1002.0)
        self.assertEqual(self.open().device, grant.device)

    async def test_storage_failure_prevents_new_work_and_keeps_failed_close_dirty(self):
        grant = self.open()
        with patch.object(self.owner._journal, 'save', side_effect=OSError('fixture')):
            with self.assertRaises(OSError):
                await self.owner.admit_mint(grant, 1000)
        self.assertEqual(self.owner._journal.load(self.account)['state']['requests'], [])
        with self.assertRaisesRegex(RuntimeError, 'recovery'):
            await self.owner.admit_mint(grant, 1000)
        self.assertFalse(self.owner.shutdown())
        self.owner = self.reopen()
        with self.assertRaisesRegex(RuntimeError, 'recovery'):
            self.open()

        self.assertFalse(self.owner.shutdown())

    async def test_failed_completion_write_cannot_publish_clean_state(self):
        grant = self.open()
        sequence = await self.owner.admit_mint(grant, 1000)
        with patch.object(self.owner._journal, 'save', side_effect=OSError('fixture')):
            with self.assertRaises(OSError):
                self.owner.finish_mint(grant, sequence, True)
        state = self.owner._journal.load(self.account)['state']
        self.assertTrue(state['pending_mint'])
        self.assertFalse(state['clean'])
        with self.assertRaisesRegex(RuntimeError, 'recovery'):
            self.owner.close_grant(grant, True)
        self.assertFalse(self.owner.shutdown())

    async def test_one_account_serializes_mints_while_other_account_can_work(self):
        first, second, other = self.open(), self.open(), self.open(self.other)
        sequence = await self.owner.admit_mint(first, 1000)
        waiting = asyncio.create_task(self.owner.admit_mint(second, 1000))
        await asyncio.sleep(0)
        self.assertFalse(waiting.done())
        other_sequence = await self.owner.admit_mint(other, 1000)
        self.owner.finish_mint(other, other_sequence, True)
        self.owner.close_grant(other, True)
        with self.assertRaisesRegex(OwnershipError, 'mint remains owned'):
            self.owner.close_grant(first, True)
        self.owner.finish_mint(first, sequence, True)
        self.clock.stamp += 0.5
        second_sequence = await asyncio.wait_for(waiting, 1)
        self.owner.close_grant(first, True)
        self.assertFalse(self.owner._journal.load(self.account)['state']['clean'])
        self.owner.finish_mint(second, second_sequence, True)
        self.owner.close_grant(second, True)
        self.assertTrue(self.owner.shutdown())

    async def test_cancelled_wait_neither_charges_nor_releases_active_mint(self):
        first, second = self.open(), self.open()
        sequence = await self.owner.admit_mint(first, 1000)
        waiting = asyncio.create_task(self.owner.admit_mint(second, 1000))
        await asyncio.sleep(0)
        waiting.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await waiting
        state = self.owner._journal.load(self.account)['state']
        self.assertEqual(state['requests'], [[first.device, [1000.0]]])
        self.owner.close_grant(second, True)
        self.assertTrue(state['pending_mint'])
        self.owner.finish_mint(first, sequence, True)
        self.owner.close_grant(first, True)
        self.assertTrue(self.owner.shutdown())

    async def test_unconfirmed_worker_retains_dirty_history_across_restart(self):
        grant = self.open()
        sequence = await self.owner.admit_mint(grant, 1000)
        self.assertFalse(self.owner.finish_mint(grant, sequence, False))
        with self.assertRaisesRegex(OwnershipError, 'unconfirmed'):
            self.open()
        with self.assertRaisesRegex(OwnershipError, 'unconfirmed'):
            self.owner.close_grant(grant, True)
        self.assertFalse(self.owner.shutdown())
        self.owner = self.reopen()
        with self.assertRaisesRegex(RuntimeError, 'recovery'):
            self.open()
        state = self.owner._journal.load(self.account)['state']
        self.assertTrue(state['pending_mint'])
        self.assertTrue(state['uncertain'])
        self.assertEqual(state['requests'], [[grant.device, [1000.0]]])
        self.assertEqual(self.open(self.other).account, self.other)

    async def test_risk_cools_only_the_saved_device_without_rotating_it(self):
        grant = self.open()
        self.owner.close_grant(grant, True, risk_refused=True)
        with self.assertRaisesRegex(OwnershipError, 'unavailable'):
            self.open()
        self.assertTrue(self.owner.shutdown())
        self.owner = self.reopen()
        state = self.owner._journal.load(self.account)['state']
        self.assertEqual(state['primary'], grant.device)
        self.assertEqual(state['cooling'], [[grant.device, 1600.0]])
        self.assertEqual(state['reserve_used'], self.before['reserve_used'])
        self.assertEqual(self.open(self.other).account, self.other)
        self.clock.stamp += 600
        self.assertEqual(self.open().device, grant.device)
