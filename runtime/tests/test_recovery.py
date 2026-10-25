"""Crash recovery preserves charges and binds every record to its old runtime."""
from copy import deepcopy
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_host.accounts.journal import RateJournal
from framerize_host.accounts.rates import AccountRates, RatePolicy
from framerize_host.accounts import runtime_owner
from test_rates import Clock


def runtime(invocation):
    return dict(boot_id='boot-one', unit='framerize.service',
                cgroup='/fixture/framerize.service', invocation=invocation * 32)


class RecoveryTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.clock = Clock()
        self.policy = RatePolicy(capacity=2, reserves=1)

    def open(self, identity):
        with patch.object(runtime_owner, 'capture', return_value=identity):
            journal = RateJournal(self.root)
        self.addCleanup(journal.close)
        return journal

    def rates(self, journal, name='account'):
        return AccountRates(journal, name, self.policy, self.clock)

    def seed(self, journal, name='account'):
        rates = self.rates(journal, name)
        device = rates.ensure_devices(1)[0]
        rates.charge_request(device)
        rates.allocate_reserve()
        rates.begin_mint()
        return deepcopy(rates.state), device

    def test_new_service_preserves_charges_devices_reserves_and_mint_cooldown(self):
        old = self.open(runtime('a'))
        before, device = self.seed(old)
        old.close()
        journal = self.open(runtime('b'))
        restored = self.rates(journal)
        self.assertFalse(restored.needs_cleanup)
        for key in ('devices', 'reserve', 'requests', 'primary', 'reserve_used', 'reserve_after'):
            self.assertEqual(restored.state[key], before[key])
        self.assertFalse(restored.state['pending_mint'])
        self.assertEqual(restored.state['last_mint'], self.clock.now())
        self.assertEqual(restored.state['cooling'][device], 1600)
        self.assertEqual(restored.ready_devices(), [])

    def test_unknown_or_same_runtime_never_rewrites_unconfirmed_history(self):
        old = self.open(runtime('a'))
        self.seed(old)
        before = old.load('account')
        old.close()
        for identity in (None, runtime('a'), dict(runtime('b'), cgroup='/other.service')):
            with self.subTest(identity=identity):
                journal = self.open(identity)
                restored = self.rates(journal)
                self.assertTrue(restored.needs_cleanup)
                with self.assertRaisesRegex(RuntimeError, 'recovery'):
                    restored.begin_mint()
                self.assertEqual(journal.load('account'), before)
                journal.close()

    def test_untouched_account_keeps_its_original_runtime_across_multiple_restarts(self):
        old = self.open(runtime('a'))
        self.seed(old, 'first')
        self.seed(old, 'later')
        later = old.load('later')
        old.close()
        second = self.open(runtime('b'))
        self.assertFalse(self.rates(second, 'first').needs_cleanup)
        self.assertEqual(second.load('later'), later)
        second.close()
        third = self.open(runtime('c'))
        self.assertTrue(third.previous_runtime_stopped('later', 'boot-one'))
        self.assertFalse(self.rates(third, 'later').needs_cleanup)

    def test_reboot_resolves_old_processes_without_forgiving_rate_restrictions(self):
        old = self.open(None)
        before, device = self.seed(old)
        old.close()
        self.clock.boot_id, self.clock.stamp = 'boot-two', 10
        journal = self.open(None)
        restored = self.rates(journal)
        self.assertFalse(restored.needs_cleanup)
        self.assertEqual(restored.state['devices'], before['devices'])
        self.assertEqual(list(restored.state['requests'][device]), [10])
        self.assertEqual(restored.state['cooling'][device], 610)
        self.assertEqual(restored.state['reserve_after'], 86410)
        self.assertIsNone(restored.allocate_reserve())

    def test_recovery_write_failure_preserves_original_record_and_owner(self):
        old = self.open(runtime('a'))
        self.seed(old)
        before = old.load('account')
        old.close()
        journal = self.open(runtime('b'))
        with patch.object(journal, 'save', side_effect=OSError('storage unavailable')):
            with self.assertRaises(OSError):
                self.rates(journal)
        self.assertEqual(journal.load('account'), before)
        self.assertTrue(journal.previous_runtime_stopped('account', 'boot-one'))

    def test_schema_one_upgrade_preserves_data_without_inventing_runtime_ownership(self):
        db = sqlite3.connect(self.root / 'rate-history.sqlite3')
        db.execute('CREATE TABLE accounts (account TEXT PRIMARY KEY, snapshot TEXT NOT NULL)')
        db.execute('PRAGMA user_version=1')
        db.execute('INSERT INTO accounts VALUES (?, ?)', ('account', json.dumps({'saved': True})))
        db.commit()
        db.close()
        journal = self.open(runtime('b'))
        self.assertEqual(journal.load('account'), {'saved': True})
        self.assertFalse(journal.previous_runtime_stopped('account', 'boot-one'))
        self.assertEqual(journal.connection.execute('PRAGMA user_version').fetchone()[0], 2)
