"""Incident regressions. Only temporary profiles and synthetic clocks."""
import fcntl
import os
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_accounts.journal import RateJournal
from framerize_accounts.rates import AccountRates, RatePolicy
from framerize_accounts import runtime_owner
from test_rates import Clock


class JournalRecoveryTests(unittest.TestCase):
    def test_unversioned_unknown_evidence_schema_is_not_adopted(self):
        path = self.root / 'rate-history.sqlite3'
        with sqlite3.connect(path) as db:
            RateJournal._create_schema(db)
            db.execute('CREATE TABLE migration_evidence (unknown TEXT)')
            db.execute('PRAGMA user_version=0')
        db.close()
        before = path.read_bytes()
        with self.assertRaisesRegex(RuntimeError, 'unsupported rate-history schema'):
            self.journal()
        self.assertEqual(path.read_bytes(), before)

    def test_later_policy_change_cannot_refill_unknown_reserves(self):
        (self.root / 'account').mkdir()
        (self.root / 'account/framerize-devices.json').write_text('["stable"]')
        (self.root / 'rate-history.sqlite3').touch()
        owner = dict(boot_id=self.clock.boot_id, invocation='a' * 32)
        with patch.object(runtime_owner, 'capture', return_value=owner), patch(
                'framerize_accounts.journal_recovery.parent_owns_sessions', return_value=True):
            journal = self.journal(recovery_policy=RatePolicy(reserves=0), clock=self.clock)
        rates = AccountRates(journal, 'account', RatePolicy(), self.clock)
        self.assertIsNone(rates.allocate_reserve())
        self.clock.stamp += 86400
        self.clock.today = '2026-10-26'
        self.assertIsNotNone(rates.allocate_reserve())

    def test_recovery_requires_the_parents_actual_exclusive_session_lock(self):
        from framerize_accounts.journal_recovery import parent_owns_sessions
        profiles = self.root / 'profiles'
        sessions = profiles / '.sessions'
        sessions.mkdir(parents=True)
        root = profiles / '.sidecar/camoufox'
        with (sessions / '.owner.lock').open('w') as lock:
            with patch.object(os, 'getppid', return_value=os.getpid()):
                self.assertFalse(parent_owns_sessions(root))
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                self.assertTrue(parent_owns_sessions(root))
                fcntl.flock(lock, fcntl.LOCK_UN)
                self.assertFalse(parent_owns_sessions(root))

    def test_reserve_change_does_not_claim_cleanup_or_rewrite_an_unknown_owner(self):
        journal = self.journal()
        rates = AccountRates(journal, 'account', clock=self.clock)
        rates.begin_mint()
        before = journal.load('account')
        restored = AccountRates(journal, 'account', RatePolicy(reserves=0), self.clock)
        self.assertTrue(restored.needs_cleanup)
        self.assertEqual(journal.load('account'), before)

    def test_failed_first_transaction_is_retryable_without_partial_schema(self):
        create = RateJournal._create_schema
        def interrupted(db):
            create(db)
            raise RuntimeError('fixture interrupted initialization')
        with patch.object(RateJournal, '_create_schema', side_effect=interrupted):
            with self.assertRaisesRegex(RuntimeError, 'fixture interrupted'):
                self.journal()
        journal = self.journal()
        self.assertEqual(journal.connection.execute('SELECT COUNT(*) FROM accounts').fetchone()[0], 0)

    def test_native_policy_uses_configured_reserve_limit_and_mint_gap(self):
        with patch.dict('os.environ', {'FRAMERIZE_RESERVE_PER_DAY': '0',
                                      'FRAMERIZE_MINT_GAP_S': '0.75'}):
            self.assertEqual(RatePolicy.from_environment(), RatePolicy(reserves=0, mint_gap=0.75))

    def test_reboot_and_reserve_change_preserve_cooldown_and_save_new_boot(self):
        journal = self.journal()
        rates = AccountRates(journal, 'account', clock=self.clock)
        device = rates.ensure_devices(1)[0]
        rates.cool_device(device)
        rates.confirm_runtime_stopped()
        self.clock.boot_id, self.clock.stamp = 'boot-two', 10
        restored = AccountRates(journal, 'account', RatePolicy(reserves=0), self.clock)
        self.assertEqual(restored.state['cooling'][device], 610)
        self.assertEqual(journal.load('account')['boot_id'], 'boot-two')
        self.assertEqual(journal.load('account')['policy']['reserves'], 0)

    def test_missing_history_without_exclusive_owner_is_not_replaced(self):
        (self.root / 'account').mkdir()
        with patch.object(runtime_owner, 'capture', return_value=None):
            with self.assertRaisesRegex(RuntimeError, 'exclusive service'):
                self.journal(recovery_policy=RatePolicy(), clock=self.clock)
        self.assertFalse((self.root / 'rate-history.sqlite3').exists())

    def test_unversioned_recognized_schema_is_adopted_without_losing_rows(self):
        path = self.root / 'rate-history.sqlite3'
        with sqlite3.connect(path) as db:
            db.execute('CREATE TABLE accounts (account TEXT PRIMARY KEY, snapshot TEXT NOT NULL)')
            db.execute('INSERT INTO accounts VALUES (?, ?)', ('account', '{"retained": true}'))
        db.close()
        journal = self.journal()
        self.assertEqual(journal.load('account'), {'retained': True})
        self.assertFalse(journal.previous_runtime_stopped('account', self.clock.boot_id))
        self.assertEqual(journal.connection.execute('PRAGMA user_version').fetchone()[0], 2)

    def test_unknown_schema_is_preserved_with_an_actionable_error(self):
        path = self.root / 'rate-history.sqlite3'
        with sqlite3.connect(path) as db:
            db.execute('CREATE TABLE valuable (value TEXT)')
            db.execute("INSERT INTO valuable VALUES ('retain')")
        db.close()
        before = path.read_bytes()
        with self.assertRaisesRegex(RuntimeError, 'preserve the database'):
            self.journal()
        self.assertEqual(path.read_bytes(), before)

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.clock = Clock()

    def journal(self, **kwargs):
        journal = RateJournal(self.root, **kwargs)
        self.addCleanup(journal.close)
        return journal

    def test_browser_return_with_reserves_disabled_preserves_migrated_debt(self):
        journal = self.journal()
        rates = AccountRates(journal, 'account', RatePolicy(), self.clock)
        device = rates.ensure_devices(1)[0]
        rates.charge_request(device)
        rates.cool_device(device)
        rates.allocate_reserve()
        rates.confirm_runtime_stopped()
        before = journal.load('account')['state']
        restored = AccountRates(journal, 'account', RatePolicy(reserves=0), self.clock)
        self.assertEqual(restored.state['reserve_used'], 1)
        self.assertIsNone(restored.allocate_reserve())
        for field in ('devices', 'requests', 'cooling', 'reserve', 'reserve_after'):
            self.assertEqual(journal.load('account')['state'][field], before[field])
        reopened = AccountRates(journal, 'account', RatePolicy(reserves=0), self.clock)
        self.assertEqual(reopened.state['reserve_used'], 1)

    def test_empty_database_from_interrupted_creation_is_completed(self):
        (self.root / 'rate-history.sqlite3').touch(mode=0o600)
        journal = self.journal()
        self.assertIsNone(journal.load('new-account'))
        self.assertEqual(journal.connection.execute('PRAGMA user_version').fetchone()[0], 2)

    def test_missing_history_recovers_identities_with_a_full_cooldown(self):
        directory = self.root / 'account'
        directory.mkdir()
        saved = directory / 'framerize-devices.json'
        saved.write_text('["stable-device"]')
        owner = dict(boot_id=self.clock.boot_id, invocation='a' * 32)
        with patch.object(runtime_owner, 'capture', return_value=owner), patch(
                'framerize_accounts.journal_recovery.parent_owns_sessions', return_value=True):
            journal = self.journal(recovery_policy=RatePolicy(), clock=self.clock)
        self.assertEqual(saved.read_text(), '["stable-device"]')
        self.assertEqual(journal.recovered_accounts, 1)
        rates = AccountRates(journal, 'account', clock=self.clock)
        self.assertEqual(rates.state['devices'], ['stable-device'])
        self.assertEqual(rates.state['requests'], {})
        self.assertIsNone(rates.allocate_reserve())
        self.assertEqual(rates.device_ready_in(), 600)
        for device in (None, 'stable-device'):
            with self.assertRaisesRegex(RuntimeError, 'budget'):
                rates.charge_request(device)
        self.clock.stamp += 600
        rates.charge_request('stable-device')
        self.assertIsNone(rates.allocate_reserve())
