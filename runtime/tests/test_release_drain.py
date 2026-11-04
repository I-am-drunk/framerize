"""Graceful activation must never time out or terminate admitted work."""
from pathlib import Path
import signal
import sys
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'scripts'))
import release_drain as drain


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

    def run_stop(self, states):
        with patch.object(drain, 'state', side_effect=states), \
             patch.object(drain, 'descendants', return_value=set()), \
             patch.object(drain.os, 'pidfd_open', return_value=91), \
             patch.object(drain.os, 'close') as close, \
             patch.object(drain.signal, 'pidfd_send_signal') as send, \
             patch.object(drain.select, 'select') as wait:
            result = drain.stop(self.service, self.executable)
        return result, send, wait, close

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


if __name__ == '__main__':
    unittest.main()
