"""Legacy import uses synthetic histories, temporary files, and inert owners."""
from copy import deepcopy
import fcntl
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_accounts import migration
from framerize_accounts.journal import RateJournal
from framerize_accounts.rates import AccountRates


class Clock:
    boot_id = 'fixture-boot'
    def now(self): return 1000.0
    def day(self): return '2026-10-25'


class MigrationTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name) / 'accounts'
        self.root.mkdir()
        (self.root / 'account').mkdir()
        (self.root / 'account/framerize-devices.json').write_text('["device"]')
        self.clock = Clock()
        self.inventory = {'account': {'devices': ['device', 'reserve'], 'reserve': ['reserve']}}
        row = dict(cooling={'device': 1200.0}, requests={'device': [850.0, 900.0]},
            last_mint=899.0, reserve_day='2026-10-25', reserve_used=1,
            primary_device='reserve', ambient_cooling=1100.0, ambient_requests=[875.0])
        self.snapshot = dict(format=1, boot_id=self.clock.boot_id, captured_monotonic=900.0,
            state=dict(generation='a' * 32, accounts={'account': row}))
        self.source = dict(boot_id=self.clock.boot_id, unit='fixture.service',
            cgroup='/user.slice/fixture.service', invocation='b' * 32)
        self.current = dict(self.source, invocation='c' * 32)
        patched = patch.object(migration.runtime_owner, 'capture', return_value=self.current)
        self.capture = patched.start()
        self.addCleanup(patched.stop)

    def install(self):
        return migration.install_legacy(self.root, self.snapshot, self.inventory,
                                        self.source, clock=self.clock)

    def test_conversion_preserves_history_without_declaring_cleanup(self):
        original = deepcopy(self.snapshot)
        record = migration.convert_legacy(self.snapshot, self.inventory, clock=self.clock)['account']
        state = record['state']
        self.assertEqual(state['devices'], ['device', 'reserve'])
        self.assertEqual(state['requests'], [['device', [850.0, 900.0]], [None, [875.0]]])
        self.assertEqual(state['cooling'], [['device', 1200.0], [None, 1100.0]])
        self.assertEqual((state['primary'], state['last_mint']), ('reserve', 899.0))
        self.assertEqual((state['reserve_used'], state['reserve_after']), (1, 87300.0))
        self.assertFalse(state['clean'])
        self.assertTrue(state['uncertain'])
        state['requests'][0][1].append(901.0)
        self.assertEqual(self.snapshot, original)

    def test_incomplete_or_incompatible_history_never_creates_a_journal(self):
        cases = []
        for key, value in [('format', True), ('boot_id', 'other'), ('captured_monotonic', 1001.0)]:
            snapshot = deepcopy(self.snapshot)
            snapshot[key] = value
            cases.append(snapshot)
        for key, value in [('requests', {'unknown': [900.0]}), ('reserve_used', True),
                           ('requests', {'device': [900.0, 850.0]}), ('last_mint', 901.0)]:
            snapshot = deepcopy(self.snapshot)
            snapshot['state']['accounts']['account'][key] = value
            cases.append(snapshot)
        snapshot = deepcopy(self.snapshot)
        snapshot['state']['accounts']['missing'] = deepcopy(snapshot['state']['accounts']['account'])
        cases.append(snapshot)
        for snapshot in cases:
            with self.subTest(snapshot=snapshot), self.assertRaises(ValueError):
                migration.install_legacy(self.root, snapshot, self.inventory,
                                         self.source, clock=self.clock)
            self.assertFalse((self.root / 'rate-history.sqlite3').exists())

    def test_installed_journal_reopens_with_original_charges_and_recovery(self):
        self.assertEqual(self.install(), 1)
        with RateJournal(self.root) as journal:
            record = journal.load('account')
            self.assertFalse(record['state']['clean'])
            rates = AccountRates(journal, 'account', clock=self.clock)
            self.assertEqual(list(rates.state['requests']['device']), [850.0, 900.0])
            self.assertEqual(list(rates.state['requests'][None]), [875.0])
            self.assertEqual(rates.state['devices'], ['device', 'reserve'])
            self.assertEqual(rates.state['cooling']['device'], 1600.0)
            self.assertEqual(rates.state['reserve_after'], 87300.0)
            self.assertEqual(rates.ready_devices(), [])
        self.assertEqual((self.root / 'rate-history.sqlite3').stat().st_mode & 0o777, 0o600)

    def test_source_owner_and_successor_changes_refuse_publication(self):
        for owner in (None, self.source, dict(self.current, unit='other.service')):
            self.capture.return_value = owner
            with self.subTest(owner=owner), self.assertRaises(ValueError):
                self.install()
            self.assertFalse((self.root / 'rate-history.sqlite3').exists())
        self.capture.side_effect = [self.current, None]
        with self.assertRaisesRegex(ValueError, 'successor runtime changed'):
            self.install()
        self.assertFalse((self.root / 'rate-history.sqlite3').exists())

    def test_existing_history_and_concurrent_owner_are_never_overwritten(self):
        with (self.root / '.rate-history.lock').open('wb') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaises(BlockingIOError):
                self.install()
        self.install()
        path = self.root / 'rate-history.sqlite3'
        before = path.read_bytes()
        with self.assertRaises(ValueError):
            self.install()
        self.assertEqual(path.read_bytes(), before)

    def test_write_failure_leaves_no_partial_history_and_allows_retry(self):
        def fail(path, *_):
            path.write_bytes(b'partial fixture database')
            raise OSError('fixture storage failure')
        with patch.object(migration, '_write_database', side_effect=fail):
            with self.assertRaises(OSError):
                self.install()
        self.assertFalse((self.root / 'rate-history.sqlite3').exists())
        self.assertEqual(list(self.root.parent.glob('.rate-migration-*')), [])
        self.assertEqual(self.install(), 1)

    def test_inventory_change_during_write_prevents_publication(self):
        write = migration._write_database
        def changed(*args):
            write(*args)
            (self.root / 'account/framerize-devices.json').write_text('["different"]')
        with patch.object(migration, '_write_database', side_effect=changed):
            with self.assertRaisesRegex(ValueError, 'device inventory differs'):
                self.install()
        self.assertFalse((self.root / 'rate-history.sqlite3').exists())

    def test_last_moment_destination_creation_is_never_overwritten(self):
        link = migration.os.link
        path = self.root / 'rate-history.sqlite3'
        def occupied(source, destination):
            path.write_bytes(b'concurrent fixture history')
            return link(source, destination)
        with patch.object(migration.os, 'link', side_effect=occupied):
            with self.assertRaises(FileExistsError):
                self.install()
        self.assertEqual(path.read_bytes(), b'concurrent fixture history')
