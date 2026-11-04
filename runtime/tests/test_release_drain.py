"""Graceful activation must never time out or terminate admitted work."""
from pathlib import Path
import io
import signal
import sys
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'scripts'))
import release_drain as drain
from release_activation import UserService


class DrainTests(unittest.TestCase):
    def setUp(self):
        self.before = dict(MainPID='41', InvocationID='owned', ActiveState='active',
            SubState='running', Result='success', ExecMainCode='0', ExecMainStatus='0',
            Restart='on-failure', NRestarts='0', ControlGroup='/fixture', NeedDaemonReload='no')
        self.after = dict(self.before, MainPID='0', ActiveState='inactive',
                          SubState='dead', ExecMainCode='1')
        self.executable = Path('/release/bin/framerize')
        self.service = Mock()
        self.service.executable.return_value = self.executable

    def run_stop(self, states, stop=None):
        with patch.object(drain, 'state', side_effect=states), \
             patch.object(drain, 'require_readiness'), \
             patch.object(drain, 'descendants', return_value=set()), \
             patch.object(drain.os, 'pidfd_open', return_value=91), \
             patch.object(drain.os, 'close') as close, \
             patch.object(drain.signal, 'pidfd_send_signal') as send, \
             patch.object(drain.select, 'select') as wait:
            result = stop() if stop else drain.stop(self.service, self.executable)
        return result, send, wait, close

    def default_service(self):
        self.service = UserService()
        self.service.command = Mock()
        self.service.executable = Mock(return_value=self.executable)
        self.service._release_executables = {self.executable}
        return self.service

    def test_default_upgrade_signals_and_drains_without_systemd_stop(self):
        service = self.default_service()
        result, send, wait, _ = self.run_stop(
            [self.before, self.before, self.before, self.after], service.stop)
        send.assert_called_once_with(91, signal.SIGTERM)
        wait.assert_called_once_with([91], [], [])
        service.command.assert_not_called()
        self.assertTrue(result['clean_exit'])

    def test_default_upgrade_never_retries_an_unconfirmed_drain(self):
        service = self.default_service()
        with patch.object(drain, 'state', return_value=self.before), \
             patch.object(drain, 'stop', side_effect=RuntimeError('unconfirmed')) as stop:
            with self.assertRaisesRegex(RuntimeError, 'unconfirmed'): service.stop()
            with self.assertRaisesRegex(RuntimeError, 'preserve current release'): service.stop()
        stop.assert_called_once()
        service.command.assert_not_called()

    def test_default_rollback_requires_no_remaining_descendants(self):
        service = self.default_service()
        service._drain_group = '/fixture'
        stopped = dict(self.after, ActiveState='failed', ControlGroup='')
        with patch.object(drain, 'state', return_value=stopped), \
             patch.object(drain, 'descendants', return_value={42}) as descendants:
            with self.assertRaisesRegex(RuntimeError, 'descendants'): service.stop()
            descendants.return_value = set()
            service.stop()
        service.command.assert_not_called()

    def test_default_upgrade_refuses_an_unvalidated_executable(self):
        service = self.default_service()
        service._release_executables = set()
        with patch.object(drain, 'state', return_value=self.before), \
             patch.object(drain, 'stop') as stop:
            with self.assertRaisesRegex(RuntimeError, 'does not belong'): service.stop()
        stop.assert_not_called()

    def test_waits_for_exact_process_without_a_forced_stop_timer(self):
        result, send, wait, close = self.run_stop([self.before, self.before, self.after])
        send.assert_called_once_with(91, signal.SIGTERM)
        wait.assert_called_once_with([91], [], [])
        close.assert_called_once_with(91)
        self.service.command.assert_not_called()
        self.assertTrue(result['clean_exit'])

    def test_changed_identity_prevents_any_signal(self):
        with patch.object(drain, 'state', side_effect=[self.before,
                dict(self.before, MainPID='42')]), \
             patch.object(drain.os, 'pidfd_open', return_value=91), \
             patch.object(drain.os, 'close'), \
             patch.object(drain.signal, 'pidfd_send_signal') as send:
            with self.assertRaisesRegex(RuntimeError, 'identity changed'):
                drain.stop(self.service, self.executable)
            send.assert_not_called()

    def test_automatic_restart_after_clean_exit_is_refused(self):
        with patch.object(drain, 'state', return_value=dict(self.before, Restart='always')), \
             patch.object(drain.os, 'pidfd_open') as opened:
            with self.assertRaisesRegex(RuntimeError, 'lifecycle settings'):
                drain.stop(self.service, self.executable)
            opened.assert_not_called()

    def test_failed_exit_or_restarted_service_cannot_allow_switch(self):
        variants = [dict(self.after, Result='exit-code', ExecMainStatus='1'),
                    dict(self.after, ExecMainCode='2'),
                    dict(self.after, NRestarts='1'),
                    dict(self.after, InvocationID='another'),
                    dict(self.after, MainPID='42', ActiveState='active')]
        for after in variants:
            with self.subTest(after=after), \
                 patch.object(drain, 'descendants', return_value=set()):
                with self.assertRaises(RuntimeError):
                    drain.clean_exit(after, self.before)

    def test_lingering_browser_blocks_switch_even_after_successful_main_exit(self):
        with patch.object(drain, 'descendants', return_value={42}):
            with self.assertRaisesRegex(RuntimeError, 'descendants remain'):
                drain.clean_exit(self.after, self.before)

    def test_uninspectable_cgroups_are_refused(self):
        for group in ('relative', '/../outside'):
            with self.assertRaises(ValueError):
                drain.descendants(group)
        def denied(root, onerror):
            onerror(PermissionError('cannot inspect cgroup'))
        with patch.object(drain.os, 'walk', side_effect=denied):
            with self.assertRaises(PermissionError):
                drain.descendants('/fixture')

    def test_unconfirmed_ownership_prevents_signaling(self):
        with patch.object(drain, 'state', return_value=self.before), \
             patch.object(drain.os, 'pidfd_open', return_value=91), \
             patch.object(drain.os, 'close'), \
             patch.object(drain, 'require_readiness', side_effect=RuntimeError('unconfirmed')), \
             patch.object(drain.signal, 'pidfd_send_signal') as send:
            with self.assertRaisesRegex(RuntimeError, 'unconfirmed'):
                drain.stop(self.service, self.executable)
            send.assert_not_called()

    def test_readiness_requires_healthy_ownership_history_and_exact_pid(self):
        for status, pid, body in ((200, '41', b'{"ready":true}'),
                                  (503, '41', b'{"ready":false}'),
                                  (200, '42', b'{"ready":true}'),
                                  (200, '41', b'{"ready":false}')):
            replies = [io.BytesIO(b'ok'), io.BytesIO(body)]
            for reply in replies:
                reply.status, reply.headers = status, {'x-framerize-pid': pid}
            self.service.http.open.side_effect = replies
            if status == 200 and pid == '41' and body == b'{"ready":true}':
                drain.require_readiness(self.service, 41)
            else:
                with self.assertRaises(RuntimeError):
                    drain.require_readiness(self.service, 41)


if __name__ == '__main__':
    unittest.main()
