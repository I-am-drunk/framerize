"""Existing-history transitions through inert service and HTTP ports."""
from contextlib import closing
import json
from pathlib import Path
import sqlite3
import unittest
import test_recovery as recovery_fixture
import test_transition as transition_fixture


def seed(path, version=2):
    path.parent.mkdir(parents=True, exist_ok=True)
    with closing(sqlite3.connect(path)) as db:
        db.execute('CREATE TABLE accounts (account TEXT PRIMARY KEY, snapshot TEXT, runtime_owner TEXT)')
        db.execute('INSERT INTO accounts VALUES (?, ?, ?)',
                   ('fixture-account', '{"untouched":true}', '{"invocation":"original"}'))
        db.execute('PRAGMA user_version=' + str(version))
        db.commit()


class ExistingHistoryTests(unittest.TestCase):
    def fixture(self, recovery=False, version=2):
        kind = recovery_fixture.RecoveryTests if recovery else transition_fixture.TransitionTests
        fixture = kind()
        self.addCleanup(fixture.doCleanups)
        fixture.setUp()
        fixture.plan['history_mode'] = 'existing-v2'
        history = Path(fixture.plan['profiles']) / '.sidecar/camoufox/rate-history.sqlite3'
        seed(history, version)
        if recovery:
            (fixture.root / 'plan.json').write_text(json.dumps(fixture.plan))
        else:
            expected = fixture.plan['expected']
            expected['binary'] = '/synthetic/original'
            expected['boot_id'] = Path('/proc/sys/kernel/random/boot_id').read_text().strip()
            config = fixture.root / 'original.conf'
            config.write_text('retained original configuration')
            expected['service_file_pins'] = {str(config): fixture.t.r.digest(config)}
            fixture.t.r.executable.return_value = expected['binary']
        fixture.history_path = history
        return fixture

    def switch(self, fixture):
        fixture.prepare_switch()
        fixture.command.side_effect = fixture.switch_command
        return fixture.t.switch(fixture.plan, fixture.saved)

    def test_existing_history_preflight_preserves_the_original_service(self):
        f = self.fixture()
        self.assertEqual(f.t.preflight(f.plan), f.saved)
        self.assertEqual(f.posts, [])
        f.command.assert_not_called()

    def test_unknown_schema_is_preserved_without_service_actions(self):
        f = self.fixture(version=99)
        before = f.history_path.read_bytes()
        with self.assertRaises(RuntimeError):
            f.t.preflight(f.plan)
        self.assertEqual(f.history_path.read_bytes(), before)
        self.assertEqual(f.posts, [])
        f.command.assert_not_called()

    def test_existing_history_mode_requires_a_present_database(self):
        f = self.fixture()
        f.history_path.unlink()
        with self.assertRaises(RuntimeError):
            f.t.preflight(f.plan)
        self.assertFalse(f.history_path.exists())
        f.command.assert_not_called()

    def test_native_startup_retains_existing_history_without_new_expiry(self):
        f = self.fixture()
        before = f.history_path.read_bytes()
        launched = self.switch(f)
        result = f.t.verify_history(f.plan, launched, {'InvocationID': 'native'})
        self.assertEqual(result, {'accounts': 1, 'history_mode': 'existing-v2'})
        self.assertEqual(f.history_path.read_bytes(), before)

    def test_changed_history_during_native_startup_is_rejected(self):
        f = self.fixture()
        launched = self.switch(f)
        with closing(sqlite3.connect(f.history_path)) as db:
            db.execute('UPDATE accounts SET snapshot=?', ('{"changed":true}',))
            db.commit()
        with self.assertRaisesRegex(RuntimeError, 'history changed during native startup'):
            f.t.verify_history(f.plan, launched, {'InvocationID': 'native'})

    def test_changed_service_configuration_still_refuses_before_mutation(self):
        f = self.fixture()
        (f.root / 'original.conf').write_text('changed configuration')
        with self.assertRaises(RuntimeError):
            f.t.preflight(f.plan)
        self.assertEqual(f.posts, [])
        f.command.assert_not_called()

    def test_early_abort_restores_only_admission_and_is_repeatable(self):
        f = self.fixture(recovery=True)
        f.arm()
        before = f.history_path.read_bytes()
        for _ in range(2):
            self.assertEqual(f.r.restore()['status'], 'aborted-original-running')
        self.assertEqual(f.commands, [])
        self.assertEqual(f.writes, [f.saved])
        self.assertEqual(f.history_path.read_bytes(), before)
        self.assertEqual(f.current['InvocationID'], 'original')
        f.r.ready.assert_not_called()
        self.assertFalse(f.dropin.exists())
        self.assertFalse(f.fallback.exists())

    def test_stopped_service_returns_to_a_history_ready_browser(self):
        f = self.fixture(recovery=True)
        f.r.remember('original-stopped')
        f.current.update(MainPID='0', ActiveState='inactive')
        f.running = None
        original = (f.root / 'original.conf').read_bytes()
        self.assertEqual(f.r.restore()['status'], 'restored-browser')
        f.r.ready.assert_called_once_with(str(f.browser), verify_history=True)
        self.assertEqual((f.root / 'original.conf').read_bytes(), original)
        self.assertTrue(f.fallback.exists())
        self.assertFalse(f.dropin.exists())

    def test_unknown_history_cannot_reopen_admission_after_an_abort(self):
        f = self.fixture(recovery=True, version=99)
        f.arm()
        with self.assertRaises(RuntimeError):
            f.r.restore()
        self.assertEqual(f.commands + f.writes, [])

    def test_native_startup_requires_its_captured_history_snapshot(self):
        f = self.fixture()
        with self.assertRaisesRegex(RuntimeError, 'snapshot is unavailable'):
            f.t.verify_history(f.plan, 1000, {'InvocationID': 'native'})

    def test_owner_change_during_abort_cannot_restore_admission(self):
        f = self.fixture(recovery=True)
        f.arm()
        def change_owner(path, value=None):
            result = f.http_json(path, value)
            f.current['InvocationID'] = 'different'
            return result
        f.r.http_json.side_effect = change_owner
        with self.assertRaisesRegex(RuntimeError, 'original service owner changed'):
            f.r.restore()
        self.assertEqual(f.commands + f.writes, [])

    def test_unknown_mode_refuses_even_an_unstarted_plan(self):
        f = self.fixture(recovery=True)
        f.plan['history_mode'] = 'unrecognized'
        (f.root / 'plan.json').write_text(json.dumps(f.plan))
        with self.assertRaises(RuntimeError):
            f.r.restore()
        self.assertEqual(f.commands + f.writes, [])


if __name__ == '__main__':
    unittest.main()
