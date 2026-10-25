"""Durability and ownership use only temporary account state."""
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

RUNTIME = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(RUNTIME))
from framerize_host.accounts.journal import RateJournal


class JournalTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)

    def test_restart_keeps_each_accounts_latest_committed_history(self):
        with RateJournal(self.root) as journal:
            journal.save('first', {'requests': [1, 2], 'cooldown': 600})
            journal.save('second', {'requests': [3]})
            journal.save('first', {'requests': [1, 2, 4], 'cooldown': 700})
        with RateJournal(self.root) as journal:
            self.assertEqual(journal.load('first'), {'requests': [1, 2, 4], 'cooldown': 700})
            self.assertEqual(journal.load('second'), {'requests': [3]})
        self.assertEqual((self.root / 'rate-history.sqlite3').stat().st_mode & 0o777, 0o600)

    def test_second_owner_is_refused_until_first_closes(self):
        with RateJournal(self.root):
            with self.assertRaises(BlockingIOError):
                RateJournal(self.root)
        with RateJournal(self.root) as journal:
            self.assertIsNone(journal.load('new'))

    def test_existing_profiles_cannot_be_mistaken_for_empty_history(self):
        (self.root / 'existing-account').mkdir()
        with self.assertRaisesRegex(RuntimeError, 'migration'):
            RateJournal(self.root)
        self.assertFalse((self.root / 'rate-history.sqlite3').exists())

    def test_untracked_account_is_refused_after_journal_creation(self):
        with RateJournal(self.root) as journal:
            (self.root / 'unknown-account').mkdir()
            with self.assertRaisesRegex(RuntimeError, 'no durable rate history'):
                journal.load('unknown-account')
            self.assertIsNone(journal.load('new-account'))

    def test_failed_serialization_preserves_last_committed_state(self):
        with RateJournal(self.root) as journal:
            journal.save('account', {'requests': [1]})
            with self.assertRaises(ValueError):
                journal.save('account', {'requests': [float('nan')]})
        with RateJournal(self.root) as journal:
            self.assertEqual(journal.load('account'), {'requests': [1]})

    def test_invalid_account_key_cannot_address_a_profile_path(self):
        with RateJournal(self.root) as journal:
            for account in ('../account', '/account', '', None):
                with self.subTest(account=account), self.assertRaises(ValueError):
                    journal.load(account)

    def test_committed_history_survives_process_exit_without_close(self):
        code = '''
import os, sys
sys.path.insert(0, sys.argv[1])
from framerize_host.accounts.journal import RateJournal
journal = RateJournal(sys.argv[2])
journal.save('account', {'requests': [10, 20], 'mint_pending': True})
os._exit(0)
'''
        result = subprocess.run([sys.executable, '-I', '-B', '-c', code,
                                 str(RUNTIME), str(self.root)], timeout=10,
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        with RateJournal(self.root) as journal:
            self.assertEqual(journal.load('account'),
                             {'requests': [10, 20], 'mint_pending': True})
