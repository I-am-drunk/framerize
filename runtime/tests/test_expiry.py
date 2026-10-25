"""Legacy expiry uses synthetic profiles, clocks and service observations."""
from dataclasses import replace
import fcntl
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_accounts import expiry, migration
from framerize_accounts.journal import RateJournal
from framerize_accounts.rates import AccountRates, RatePolicy


class Clock:
    boot_id, value, today = 'fixture-boot', 1000.0, '2026-10-25'
    def now(self): return self.value
    def day(self): return self.today


class ExpiryTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name) / 'accounts'
        (self.root / 'account').mkdir(parents=True)
        self.devices = self.root / 'account/framerize-devices.json'
        self.devices.write_text('["first", "second"]')
        self.clock = Clock()
        self.source = dict(boot_id=self.clock.boot_id, unit='fixture.service',
            cgroup='/user.slice/fixture.service', invocation='a' * 32)
        self.current = dict(self.source, invocation='b' * 32)
        self.capture = self.enterContext(patch.object(expiry.runtime_owner,
            'capture', return_value=self.current))

    def witness(self, **kwargs):
        witness = expiry.ExpiryWitness(self.root, self.source, clock=self.clock, **kwargs)
        self.addCleanup(witness.close)
        return witness

    def test_expiry_preserves_devices_and_distinguishes_unknown_history(self):
        witness = self.witness()
        self.assertEqual(witness.remaining(), 600.0)
        self.clock.value = 1599.9
        with self.assertRaisesRegex(ValueError, 'not expired'):
            witness.install()
        self.assertFalse((self.root / 'rate-history.sqlite3').exists())
        self.clock.value = 1600.0
        self.assertEqual(witness.install(), 1)
        with self.assertRaises(BlockingIOError):
            RateJournal(self.root)
        witness.close()
        with RateJournal(self.root) as journal:
            rates = AccountRates(journal, 'account', clock=self.clock)
            self.assertEqual(rates.ready_devices(), ['first', 'second'])
            self.assertIsNone(rates.state['primary'])
            self.assertEqual(rates.state['requests'], {})
            evidence = json.loads(journal.connection.execute(
                'SELECT record FROM migration_evidence').fetchone()[0])
            self.assertEqual(evidence['prior_usage'], 'unknown-expired')
            self.assertEqual(evidence['source_runtime'], self.source)
            self.assertEqual((evidence['started_at'], evidence['completed_at']), (1000, 1600))
            rates.charge_request('first')
        with RateJournal(self.root) as journal:
            rates = AccountRates(journal, 'account', clock=self.clock)
            self.assertEqual(list(rates.state['requests']['first']), [1600.0])
        self.assertEqual(self.devices.read_text(), '["first", "second"]')

    def test_unknown_daily_usage_stays_reserved_despite_calendar_jump(self):
        witness = self.witness()
        self.clock.value = 1600
        witness.install()
        witness.close()
        with RateJournal(self.root) as journal:
            rates = AccountRates(journal, 'account', clock=self.clock)
            self.assertEqual(rates.state['reserve_used'], 8)
            self.assertIsNone(rates.allocate_reserve())
            self.clock.today = '2026-10-30'
            self.clock.value = 87399
            self.assertIsNone(rates.allocate_reserve())
        with RateJournal(self.root) as journal:
            rates = AccountRates(journal, 'account', clock=self.clock)
            self.assertIsNone(rates.allocate_reserve())
            self.clock.value = 87400
            self.assertIsNotNone(rates.allocate_reserve())
            self.assertEqual(rates.state['reserve_used'], 1)

    def test_accounts_keep_distinct_devices_and_new_charges(self):
        (self.root / 'other').mkdir()
        (self.root / 'other/framerize-devices.json').write_text('["other-device"]')
        witness = self.witness()
        self.clock.value = 1600
        self.assertEqual(witness.install(), 2)
        witness.close()
        with RateJournal(self.root) as journal:
            first = AccountRates(journal, 'account', clock=self.clock)
            other = AccountRates(journal, 'other', clock=self.clock)
            self.assertEqual(first.ready_devices(), ['first', 'second'])
            self.assertEqual(other.ready_devices(), ['other-device'])
            first.charge_request('first')
            first.cool_device('first')
            self.assertEqual(other.state['requests'], {})
            self.assertEqual(other.ready_devices(), ['other-device'])
            self.assertIsNone(other.allocate_reserve())
            self.assertEqual(journal.load('account')['state']['requests'],
                             [['first', [1600.0]]])

    def test_stricter_policy_extends_the_wait(self):
        policy = replace(RatePolicy(), window=900.0)
        witness = self.witness(policy=policy)
        self.clock.value = 1600
        self.assertEqual(witness.remaining(), 300)
        with self.assertRaisesRegex(ValueError, 'not expired'):
            witness.install()
        self.clock.value = 1900
        self.assertEqual(witness.install(), 1)

    def test_closing_loses_elapsed_credit_and_reopening_starts_again(self):
        old = self.witness()
        self.clock.value = 1599
        old.close()
        with self.assertRaisesRegex(ValueError, 'closed'):
            old.install()
        self.assertEqual(self.witness().remaining(), 600)

    def test_unknown_or_same_runtime_cannot_begin_expiry(self):
        for owner in (None, self.source, dict(self.current, unit='other.service')):
            self.capture.return_value = owner
            with self.subTest(owner=owner), self.assertRaises(ValueError):
                self.witness()
        self.capture.return_value = self.current
        self.assertEqual(self.witness().remaining(), 600)

    def test_lost_runtime_proof_closes_witness_permanently(self):
        witness = self.witness()
        self.clock.value = 1600
        self.capture.return_value = None
        with self.assertRaisesRegex(ValueError, 'runtime changed'):
            witness.install()
        self.capture.return_value = self.current
        with self.assertRaisesRegex(ValueError, 'closed'):
            witness.install()
        self.assertEqual(self.witness().remaining(), 600)

    def test_clock_boot_and_process_changes_cannot_reuse_elapsed_time(self):
        for change in ('clock', 'boot', 'process'):
            self.clock = Clock()
            witness = self.witness()
            self.clock.value = 1600
            self.assertEqual(witness.remaining(), 0)
            if change == 'clock':
                self.clock.value = 1599
            if change == 'boot':
                self.clock.boot_id = 'other-boot'
            pid = expiry.os.getpid() + (1 if change == 'process' else 0)
            with self.subTest(change=change), patch.object(expiry.os, 'getpid', return_value=pid):
                with self.assertRaises(ValueError):
                    witness.install()
            self.assertFalse((self.root / 'rate-history.sqlite3').exists())

    def test_competing_owner_and_existing_journal_are_refused(self):
        with (self.root / '.rate-history.lock').open('wb') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaises(BlockingIOError):
                self.witness()
        witness = self.witness()
        self.clock.value = 1600
        witness.install()
        witness.close()
        path = self.root / 'rate-history.sqlite3'
        original = path.read_bytes()
        with self.assertRaises(ValueError):
            self.witness()
        self.assertEqual(path.read_bytes(), original)

    def test_invalid_or_missing_saved_identities_are_not_replaced(self):
        for raw in ('[]', '[true]', '["duplicate", "duplicate"]', '{}', '['):
            self.devices.write_text(raw)
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                self.witness()
            self.assertEqual(self.devices.read_text(), raw)
        self.devices.unlink()
        with self.assertRaises(ValueError):
            self.witness()
        self.assertFalse((self.root / 'rate-history.sqlite3').exists())

    def test_inventory_change_before_publish_refuses_and_releases_lock(self):
        witness = self.witness()
        self.clock.value = 1600
        write = migration._write_database
        def changed(*args):
            write(*args)
            self.devices.write_text('["different"]')
        with patch.object(migration, '_write_database', side_effect=changed):
            with self.assertRaisesRegex(ValueError, 'inventory differs'):
                witness.install()
        self.assertEqual(self.witness().remaining(), 600)

    def test_ownership_is_checked_again_after_database_write(self):
        write = migration._write_database
        for change in ('runtime', 'lock'):
            witness = self.witness()
            self.clock.value += 600
            def changed(*args):
                write(*args)
                if change == 'runtime':
                    self.capture.return_value = None
                else:
                    lock = self.root / '.rate-history.lock'
                    lock.rename(self.root.parent / 'retired-lock')
                    lock.touch()
            with self.subTest(change=change), patch.object(
                    migration, '_write_database', side_effect=changed):
                with self.assertRaises(ValueError):
                    witness.install()
            self.assertFalse((self.root / 'rate-history.sqlite3').exists())
            self.capture.return_value = self.current

    def test_write_failure_leaves_no_journal_or_reusable_witness(self):
        witness = self.witness()
        self.clock.value = 1600
        def failed(path, *_):
            path.write_bytes(b'partial fixture database')
            raise OSError('fixture storage failure')
        with patch.object(migration, '_write_database', side_effect=failed):
            with self.assertRaises(OSError):
                witness.install()
        self.assertFalse((self.root / 'rate-history.sqlite3').exists())
        self.assertEqual(list(self.root.parent.glob('.rate-migration-*')), [])
        with self.assertRaisesRegex(ValueError, 'closed'):
            witness.install()
        self.assertEqual(self.witness().remaining(), 600)
