"""Native migration fixtures never access an installed account or SDK."""
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_accounts import runtime_owner
from framerize_accounts.expiry import ExpiryWitness
from framerize_accounts.journal import RateJournal
from framerize_accounts.rates import AccountRates
from framerize_native.migration import expire_legacy
from framerize_native.owner import LedgerOwner, OwnershipError


class Clock:
    boot_id, value = 'fixture-boot', 1000.0
    def now(self): return self.value
    def day(self): return '2026-10-25'


class MigrationTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name) / 'history'
        self.account = str(uuid.UUID(int=1))
        (self.root / self.account).mkdir(parents=True)
        self.devices = self.root / self.account / 'framerize-devices.json'
        self.devices.write_text('["first", "second"]')
        self.clock = Clock()
        self.source = dict(boot_id=self.clock.boot_id, unit='fixture.service',
            cgroup='/user.slice/fixture.service', invocation='a' * 32)
        self.previous = self.root.parent / 'previous.json'
        self.previous.write_text(json.dumps(self.source))
        self.enterContext(patch.object(runtime_owner, 'capture',
            return_value=dict(self.source, invocation='b' * 32)))

    def advance(self, seconds):
        self.assertGreater(seconds, 0)
        self.assertLessEqual(seconds, 1)
        self.clock.value += seconds

    def migrate(self, **kwargs):
        return expire_legacy(self.root, self.previous, clock=self.clock,
                             sleep=kwargs.get('sleep', self.advance))

    async def test_expiry_then_native_mint_keeps_devices_and_durable_charges(self):
        self.assertEqual(self.migrate(), 1)
        self.assertEqual(self.clock.now(), 1600)
        with RateJournal(self.root) as journal:
            self.assertIsNone(journal.load(self.account)['state']['primary'])
            provenance = json.loads(journal.connection.execute(
                'SELECT record FROM migration_evidence').fetchone()[0])
            self.assertEqual(provenance['prior_primary'], 'unknown')
        owner = LedgerOwner(self.root, clock=self.clock)
        self.addCleanup(owner.shutdown)
        grant = owner.open(self.account, str(uuid.uuid4()))
        self.assertEqual(grant.device, 'first')
        sequence = await owner.admit_mint(grant, 1000)
        self.assertTrue(owner.finish_mint(grant, sequence, True))
        self.assertTrue(owner.close_grant(grant, True))
        self.assertTrue(owner.shutdown())
        with RateJournal(self.root) as journal:
            rates = AccountRates(journal, self.account, clock=self.clock)
            self.assertEqual(rates.state['primary'], 'first')
            self.assertEqual(list(rates.state['requests']['first']), [1600.0])
            self.assertEqual(rates.state['reserve_used'], 8)
            self.assertIsNone(rates.allocate_reserve())
        self.assertEqual(self.devices.read_text(), '["first", "second"]')

    async def test_known_cooled_primary_is_not_replaced_by_another_device(self):
        self.migrate()
        with RateJournal(self.root) as journal:
            rates = AccountRates(journal, self.account, clock=self.clock)
            self.assertEqual(rates.select_primary(), 'first')
            rates.cool_device('first')
            rates.confirm_runtime_stopped()
        owner = LedgerOwner(self.root, clock=self.clock)
        self.addCleanup(owner.shutdown)
        with self.assertRaisesRegex(OwnershipError, 'saved device is unavailable'):
            owner.open(self.account, str(uuid.uuid4()))
        owner.shutdown()
        with RateJournal(self.root) as journal:
            rates = AccountRates(journal, self.account, clock=self.clock)
            self.assertEqual(rates.state['primary'], 'first')
            self.assertEqual(rates.ready_devices(), ['second'])

    async def test_cancellation_does_not_publish_or_retain_elapsed_credit(self):
        def cancel(seconds):
            self.advance(seconds)
            raise KeyboardInterrupt
        with self.assertRaises(KeyboardInterrupt):
            self.migrate(sleep=cancel)
        self.assertFalse((self.root / 'rate-history.sqlite3').exists())
        with ExpiryWitness(self.root, self.source, clock=self.clock) as witness:
            self.assertEqual(witness.remaining(), 600)

    async def test_invalid_metadata_never_creates_history(self):
        for raw in ('{}', '[', ' ' * 4097):
            self.previous.write_text(raw)
            with self.subTest(raw=raw[:10]), self.assertRaises(ValueError):
                self.migrate()
            self.assertFalse((self.root / 'rate-history.sqlite3').exists())
        self.assertEqual(self.devices.read_text(), '["first", "second"]')
