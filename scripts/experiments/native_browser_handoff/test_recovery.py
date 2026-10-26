"""Inert recovery regressions. No service or network operation is performed."""
import fcntl
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch


class RecoveryTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        spec = importlib.util.spec_from_file_location('recovery', Path(__file__).with_name('recovery.py'))
        self.r = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.r)
        self.r.HERE = self.root
        self.r.CGROUP_ROOT = self.root / 'cgroup'
        self.events = self.r.CGROUP_ROOT / 'user.slice/test.service/cgroup.events'
        self.events.parent.mkdir(parents=True)
        self.events.write_text('populated 0\n')
        self.original, self.native, self.browser = [self.root / n for n in ('old', 'native', 'browser')]
        for path in (self.original, self.native, self.browser):
            path.write_bytes(path.name.encode())
        self.dropin, self.fallback = self.root / 'native-dropin', self.root / 'browser-dropin'
        self.history = self.root / 'profiles/.sidecar/camoufox/rate-history.sqlite3'
        self.history.parent.mkdir(parents=True)
        self.saved = {'eligible_accounts': 'all', 'active_capacity': 2}
        for name, value in [('native.conf', 'native'), ('browser.conf', 'browser'),
                            ('pool-config.json', json.dumps(self.saved)), ('original.conf', 'old')]:
            (self.root / name).write_text(value)
        self.plan = dict(version=1, profiles=str(self.root / 'profiles'),
            native=str(self.native), browser=str(self.browser), dropin=str(self.dropin),
            return_dropin=str(self.fallback),
            expected=dict(binary=str(self.original), binary_sha256=self.r.digest(self.original),
                service={'ControlGroup': '/user.slice/test.service', 'InvocationID': 'original'},
                service_file_pins={str(self.root / 'original.conf'): self.r.digest(self.root / 'original.conf')}),
            binaries={str(p): self.r.digest(p) for p in (self.native, self.browser)},
            artifacts={n: self.r.digest(self.root / n) for n in
                ('native.conf', 'browser.conf', 'pool-config.json')})
        (self.root / 'plan.json').write_text(json.dumps(self.plan))
        self.current = dict(MainPID='123', ActiveState='active', InvocationID='original')
        self.running = str(self.original)
        self.pool = dict(self.saved, eligible_accounts=[])
        self.commands, self.writes = [], []
        self.stop_action = self.start_action = None
        for name, action in [('status', lambda: dict(self.current)),
                ('executable', lambda _: self.running), ('command', self.command),
                ('ready', self.ready), ('http_json', self.http_json)]:
            mocked = patch.object(self.r, name, side_effect=action)
            mocked.start()
            self.addCleanup(mocked.stop)
        clock = patch.object(self.r.time, 'monotonic', side_effect=iter(range(0, 3000, 30)))
        self.clock = clock.start()
        self.addCleanup(clock.stop)

    def command(self, *args):
        self.commands.append(args)
        if args[0] == 'stop':
            self.current.update(MainPID='0', ActiveState='inactive')
            self.running = None
            if self.stop_action:
                self.stop_action()
        if args[0] == 'start':
            if self.start_action:
                self.start_action()
            self.running = str(self.browser if self.fallback.exists() else self.original)
            self.current.update(MainPID='456', ActiveState='active', InvocationID='returned')

    def ready(self, expected):
        self.assertEqual(self.running, expected)
        self.assertEqual(self.current['ActiveState'], 'active')
        return int(self.current['MainPID'])

    def http_json(self, path, value=None):
        self.assertEqual(path, '/api/pool/config')
        if value is not None:
            self.writes.append(value)
            self.pool = value
        return self.pool

    def arm(self, native=False):
        self.r.remember('fenced')
        if native:
            self.running = str(self.native)
            self.current['InvocationID'] = 'native'
            self.dropin.write_text('native')
            self.fallback.write_text('browser')

    def migrate(self):
        self.history.write_bytes(b'opaque durable history')

    def test_stopped_waits_for_descendant_exit_after_systemd_stop(self):
        self.current.update(MainPID='0', ActiveState='inactive')
        self.events.write_text('populated 1\n')
        self.clock.side_effect = [0, 0.1]
        with patch.object(self.r.time, 'sleep', side_effect=lambda _: self.events.unlink()) as sleep:
            self.r.stopped(self.plan)
        sleep.assert_called_once()
        self.assertEqual(self.commands, [])

    def test_stopped_waits_for_systemd_to_finish_deactivation(self):
        self.current['ActiveState'] = 'deactivating'
        self.clock.side_effect = [0, 0.1]
        def finish(_):
            self.current.update(MainPID='0', ActiveState='inactive')
        with patch.object(self.r.time, 'sleep', side_effect=finish) as sleep:
            self.r.stopped(self.plan)
        sleep.assert_called_once()
        self.assertEqual(self.commands, [])

    def test_stopped_keeps_the_descendant_guard_at_its_deadline(self):
        self.current.update(MainPID='0', ActiveState='inactive')
        self.events.write_text('populated 1\n')
        self.clock.side_effect = [0, 9, 10]
        with patch.object(self.r.time, 'sleep') as sleep:
            with self.assertRaisesRegex(RuntimeError, 'descendants remain') as failure:
                self.r.stopped(self.plan)
        sleep.assert_called_once()
        self.assertEqual(type(failure.exception).__name__, 'ServiceDescendantsRemain')
        self.assertEqual(self.commands, [])

    def test_an_unexpected_restart_cannot_pass_the_stopped_guard(self):
        self.current.update(MainPID='0', ActiveState='inactive')
        self.events.write_text('populated 1\n')
        self.clock.side_effect = [0, 0.1, 10]
        def restart(_):
            self.current.update(MainPID='999', ActiveState='active')
            self.events.write_text('populated 0\n')
        with patch.object(self.r.time, 'sleep', side_effect=restart):
            with self.assertRaisesRegex(RuntimeError, 'shutdown is unconfirmed') as failure:
                self.r.stopped(self.plan)
        self.assertEqual(type(failure.exception).__name__, 'ServiceShutdownUnconfirmed')
        self.assertEqual(self.commands, [])

    def test_unstarted_plan_does_not_touch_service(self):
        self.assertEqual(self.r.restore(), {'status': 'not-started'})
        self.assertEqual(self.commands + self.writes, [])

    def test_fenced_original_restores_pool_without_restart(self):
        self.arm()
        self.assertEqual(self.r.restore()['status'], 'restored-original')
        self.assertEqual(self.commands, [])
        self.assertEqual(self.writes, [self.saved])

    def test_stop_intent_recovers_original_only_after_join(self):
        self.arm()
        self.r.remember('stopping-original')
        self.assertEqual(self.r.restore()['status'], 'restored-original')
        self.assertEqual(self.commands, [('stop', self.r.UNIT), ('daemon-reload',), ('start', self.r.UNIT)])
        self.assertEqual(self.pool, self.saved)

    def test_interrupted_original_stop_with_no_main_pid_is_joined(self):
        self.arm()
        self.r.remember('stopping-original')
        self.current.update(MainPID='0', ActiveState='deactivating')
        self.running = None
        self.assertEqual(self.r.restore()['status'], 'restored-original')
        self.assertEqual(self.commands, [('stop', self.r.UNIT), ('daemon-reload',), ('start', self.r.UNIT)])

    def test_stopped_original_is_recoverable_before_any_override(self):
        for phase, invocation in [('stopping-original', 'original'),
                                  ('original-stopped', 'original'), ('original-stopped', '')]:
            with self.subTest(phase=phase, invocation=invocation):
                self.r.remember(phase)
                self.current.update(MainPID='0', ActiveState='inactive', InvocationID=invocation)
                self.running = None
                self.commands.clear()
                self.assertEqual(self.r.restore()['status'], 'restored-original')
                self.assertEqual(self.commands, [('daemon-reload',), ('start', self.r.UNIT)])

    def test_partially_installed_overrides_are_removed_before_original_return(self):
        for include_native in (False, True):
            with self.subTest(include_native=include_native):
                self.r.remember('original-stopped')
                self.current.update(MainPID='0', ActiveState='inactive', InvocationID='original')
                self.running = None
                self.fallback.write_text('browser')
                if include_native:
                    self.dropin.write_text('native')
                self.commands.clear()
                self.assertEqual(self.r.restore()['status'], 'restored-original')
                self.assertEqual(self.commands, [('daemon-reload',), ('start', self.r.UNIT)])
                self.assertFalse(self.dropin.exists() or self.fallback.exists())

    def test_stop_intent_does_not_allow_an_unrelated_original_invocation(self):
        for pid, active in [('123', 'active'), ('0', 'deactivating'), ('0', 'inactive')]:
            with self.subTest(pid=pid):
                self.r.remember('stopping-original')
                self.current.update(MainPID=pid, ActiveState=active, InvocationID='unrelated')
                with self.assertRaises(RuntimeError):
                    self.r.restore()
                self.assertEqual(self.commands + self.writes, [])

    def test_stopped_original_with_descendants_cannot_be_restarted(self):
        self.r.remember('original-stopped')
        self.current.update(MainPID='0', ActiveState='inactive')
        self.running = None
        self.events.write_text('populated 1\n')
        with self.assertRaises(RuntimeError):
            self.r.restore()
        self.assertEqual(self.commands + self.writes, [])

    def test_missing_original_without_stop_intent_remains_refused(self):
        self.arm()
        self.current.update(MainPID='0', ActiveState='inactive')
        self.running = None
        with self.assertRaisesRegex(RuntimeError, 'original service is missing'):
            self.r.restore()
        self.assertEqual(self.commands + self.writes, [])

    def test_lost_original_stop_reply_is_recoverable(self):
        self.r.remember('stopping-original')
        def lost_reply():
            raise TimeoutError('synthetic stop reply loss')
        self.stop_action = lost_reply
        with self.assertRaises(TimeoutError):
            self.r.restore()
        self.assertEqual(json.loads((self.root / 'state.json').read_text())['phase'], 'stopping-original')
        self.assertEqual(self.commands, [('stop', self.r.UNIT)])
        self.stop_action = None
        self.commands.clear()
        self.assertEqual(self.r.restore()['status'], 'restored-original')
        self.assertEqual(self.commands, [('daemon-reload',), ('start', self.r.UNIT)])

    def test_interrupted_reload_restores_original_from_stopped_service(self):
        self.r.remember('switching')
        self.current.update(MainPID='0', ActiveState='inactive', InvocationID='')
        self.running = None
        self.dropin.write_text('native')
        self.fallback.write_text('browser')
        self.assertEqual(self.r.restore()['status'], 'restored-original')
        self.assertEqual(self.commands, [('daemon-reload',), ('start', self.r.UNIT)])
        self.assertFalse(self.dropin.exists() or self.fallback.exists())
        self.assertEqual(self.pool, self.saved)

    def test_unmigrated_native_returns_to_original(self):
        self.arm(native=True)
        result = self.r.restore()
        self.assertEqual(result['status'], 'restored-original')
        self.assertEqual(self.commands, [('stop', self.r.UNIT), ('daemon-reload',), ('start', self.r.UNIT)])
        self.assertFalse(self.dropin.exists() or self.fallback.exists() or self.history.exists())
        self.assertEqual(self.pool, self.saved)

    def test_migrated_native_preserves_history_and_persistent_browser_return(self):
        self.arm(native=True)
        self.migrate()
        result = self.r.restore()
        self.assertEqual(result['status'], 'restored-browser')
        self.assertEqual(self.history.read_bytes(), b'opaque durable history')
        self.assertFalse(self.dropin.exists())
        self.assertEqual(self.fallback.read_text(), 'browser')
        self.assertEqual(self.pool, self.saved)

    def test_migration_finishing_during_stop_changes_the_return_target(self):
        self.arm(native=True)
        self.stop_action = self.migrate
        self.assertEqual(self.r.restore()['status'], 'restored-browser')
        self.assertEqual(self.running, str(self.browser))

    def test_repeated_browser_recovery_does_not_restart_or_rewrite_pool(self):
        self.arm(native=True)
        self.migrate()
        first = self.r.restore()
        self.commands.clear()
        self.writes.clear()
        self.assertEqual(self.r.restore(), first)
        self.assertEqual(self.commands, [('daemon-reload',)])
        self.assertEqual(self.writes, [])

    def test_unrelated_executable_is_never_stopped(self):
        self.arm(native=True)
        self.running = '/unrelated/service'
        with self.assertRaisesRegex(RuntimeError, 'another service executable'):
            self.r.restore()
        self.assertEqual(self.commands + self.writes, [])

    def test_changed_service_configuration_prevents_any_operation(self):
        self.arm(native=True)
        (self.root / 'original.conf').write_text('changed')
        with self.assertRaisesRegex(RuntimeError, 'original service configuration changed'):
            self.r.restore()
        self.assertEqual(self.commands + self.writes, [])

    def test_changed_saved_pool_prevents_any_operation(self):
        self.arm(native=True)
        (self.root / 'pool-config.json').write_text('{}')
        with self.assertRaisesRegex(RuntimeError, 'transition artifact changed'):
            self.r.restore()
        self.assertEqual(self.commands + self.writes, [])

    def test_remaining_descendants_prevent_return_and_preserve_dropins(self):
        self.arm(native=True)
        self.stop_action = lambda: self.events.write_text('populated 1\n')
        with self.assertRaisesRegex(RuntimeError, 'descendants remain'):
            self.r.restore()
        self.assertEqual(self.commands, [('stop', self.r.UNIT)])
        self.assertTrue(self.dropin.exists() and self.fallback.exists())
        self.assertEqual(self.writes, [])

    def test_live_main_pid_prevents_return(self):
        self.arm(native=True)
        self.stop_action = lambda: self.current.update(MainPID='789', ActiveState='active')
        with self.assertRaisesRegex(RuntimeError, 'shutdown is unconfirmed'):
            self.r.restore()
        self.assertEqual(self.commands, [('stop', self.r.UNIT)])
        self.assertTrue(self.dropin.exists())

    def test_interrupted_original_start_can_be_retried_after_dropin_removal(self):
        self.arm(native=True)
        def fail():
            raise OSError('synthetic start failure')
        self.start_action = fail
        with self.assertRaises(OSError):
            self.r.restore()
        self.assertFalse(self.dropin.exists() or self.fallback.exists())
        self.start_action = None
        self.commands.clear()
        self.assertEqual(self.r.restore()['status'], 'restored-original')
        self.assertEqual(self.commands, [('daemon-reload',), ('start', self.r.UNIT)])

    def test_unexpected_history_symlink_cannot_reenable_original(self):
        self.arm(native=True)
        self.history.symlink_to(self.root / 'missing')
        with self.assertRaisesRegex(RuntimeError, 'history is not a regular file'):
            self.r.restore()
        self.assertEqual(self.commands, [('stop', self.r.UNIT)])
        self.assertTrue(self.dropin.exists() and self.fallback.exists())

    def test_lost_pool_update_retries_without_stopping_returned_browser(self):
        self.arm(native=True)
        self.migrate()
        with patch.object(self.r, 'http_json', side_effect=OSError('lost update')):
            with self.assertRaises(OSError):
                self.r.restore()
        self.commands.clear()
        self.assertEqual(self.r.restore()['status'], 'restored-browser')
        self.assertNotIn(('stop', self.r.UNIT), self.commands)
        self.assertNotIn(('start', self.r.UNIT), self.commands)
        self.assertEqual(self.pool, self.saved)

    def test_recovery_lock_excludes_a_second_caller(self):
        self.arm(native=True)
        with (self.root / 'recovery.lock').open('w') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaises(BlockingIOError):
                self.r.restore()
        self.assertEqual(self.commands + self.writes, [])

    def test_original_invocation_change_prevents_pool_overwrite(self):
        self.arm()
        self.current['InvocationID'] = 'unrelated-restart'
        with self.assertRaisesRegex(RuntimeError, 'original service owner changed'):
            self.r.restore()
        self.assertEqual(self.commands + self.writes, [])

    def test_unrelated_pool_change_is_not_overwritten(self):
        self.arm()
        self.pool = {'eligible_accounts': 'all', 'active_capacity': 3}
        with self.assertRaisesRegex(RuntimeError, 'outside this transition'):
            self.r.restore()
        self.assertEqual(self.commands + self.writes, [])


if __name__ == '__main__':
    unittest.main()
