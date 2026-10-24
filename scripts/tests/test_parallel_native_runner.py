from contextlib import redirect_stdout
import hashlib
import io
import json
import os
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
from native_check_fixture import Scenario, quiet_pool
from parallel_native_check import runner


class RunnerTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.plan = self.root / 'plan.json'
        self.plan.write_text('{}')
        self.digest = hashlib.sha256(b'{}').hexdigest()
        previous = os.umask(0o077)
        self.addCleanup(os.umask, previous)

    def invoke(self, name):
        output = self.root / name
        args = ['check', '--plan', str(self.plan), '--plan-sha256', self.digest,
                '--report', str(output), '--execute']
        with patch('sys.argv', args), redirect_stdout(io.StringIO()):
            result = runner.main()
        return result, json.loads(output.read_text())

    def binding(self):
        return SimpleNamespace(root=self.root, pid=os.getpid(),
                               plan={'invocation_id': 'a' * 32, 'events_path': ''})

    def test_different_report_name_does_not_allow_generation_replay(self):
        events = SimpleNamespace(completed=[], send_invocations=[], send_snapshots=[],
                                 bind_writer=lambda pid: None, close=lambda: None)
        with patch.object(runner, 'Binding', return_value=self.binding()):
            with patch.object(runner, 'Events', return_value=events):
                with patch.object(runner, 'run') as run:
                    run.side_effect = lambda _b, _e, report: report.update(accepted=True)
                    self.assertEqual(self.invoke('first.json')[0], 0)
                    code, report = self.invoke('second.json')
        self.assertEqual(code, 1)
        self.assertEqual(report['error_code'], 'FileExistsError')
        self.assertEqual(report['post_calls'], 0)
        run.assert_called_once()

    def test_preflight_exception_saves_no_raw_error_or_request(self):
        with patch.object(runner, 'Binding', side_effect=RuntimeError('private-value')):
            with patch.object(runner, 'run') as run:
                code, report = self.invoke('failed.json')
        self.assertEqual(code, 1)
        self.assertEqual(report['error_code'], 'RuntimeError')
        self.assertEqual(report['post_calls'], 0)
        self.assertNotIn('private-value', json.dumps(report))
        run.assert_not_called()

    def test_stale_daily_log_refuses_before_any_request_or_cleanup_wait(self):
        logs = self.root / 'profiles/.logs'
        logs.mkdir(parents=True)
        old = logs / 'events-2026-10-23.ndjson'
        old.write_text('')
        self.enterContext((logs / 'events-2026-10-24.ndjson').open('ab'))
        binding = self.binding()
        binding.plan['events_path'] = str(old)
        with patch.object(runner, 'Binding', return_value=binding):
            with patch.object(runner, 'run') as run:
                with patch.object(runner, 'observe_failed_cleanup') as observe:
                    code, report = self.invoke('stale-log.json')
        self.assertEqual(code, 1)
        self.assertEqual(report['error_code'], 'event_writer_mismatch')
        self.assertEqual(report['post_calls'], 0)
        self.assertNotIn('cleanup_after_error', report)
        run.assert_not_called()
        observe.assert_not_called()

    def test_failure_still_observes_existing_cleanup_without_retry(self):
        scenario = Scenario()
        scenario.binding.root = self.root
        scenario.binding.pid = os.getpid()
        scenario.binding.plan = {'invocation_id': 'b' * 32, 'events_path': ''}
        scenario.events.close = lambda: None
        scenario.events.bind_writer = lambda pid: None

        def fail(binding, events, report):
            scenario.send(binding, True, report).close(cancel=True)
            raise RuntimeError('private-downstream-failure')

        with patch.object(runner, 'Binding', return_value=scenario.binding):
            with patch.object(runner, 'Events', return_value=scenario.events):
                with patch.object(runner, 'run', side_effect=fail):
                    with patch('parallel_native_check.client.pool', return_value=quiet_pool()):
                        code, report = self.invoke('request-failed.json')
        self.assertEqual(code, 1)
        self.assertEqual(report['post_calls'], 1)
        self.assertTrue(report['cleanup_after_error']['confirmed'])
        self.assertNotIn('private-downstream-failure', json.dumps(report))
