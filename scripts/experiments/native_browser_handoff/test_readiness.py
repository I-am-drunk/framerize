"""A successful HTTP listener alone cannot certify a browser return."""
from contextlib import nullcontext
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import Mock, patch


class ReturnReadinessTests(unittest.TestCase):
    def setUp(self):
        spec = importlib.util.spec_from_file_location(
            'recovery', Path(__file__).with_name('recovery.py'))
        self.r = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.r)

    def probe(self, payload, pid='123'):
        reply = Mock(status=200, headers={'x-framerize-pid': pid})
        reply.read.return_value = payload
        http = Mock()
        http.open.return_value = nullcontext(reply)
        patches = [('status', lambda: {'MainPID': '123', 'ActiveState': 'active'}),
                   ('executable', lambda _: '/fixture/browser')]
        for name, value in patches:
            mocked = patch.object(self.r, name, value)
            mocked.start()
            self.addCleanup(mocked.stop)
        return http

    def test_http_success_without_verified_history_is_not_restored(self):
        for payload in (b'ok', b'{"ready": false}', b'{"ready": 1}', b'[]', b'x' * 4097):
            with self.subTest(payload=payload[:40]):
                http = self.probe(payload)
                with patch.object(self.r.urllib.request, 'build_opener', return_value=http), patch.object(
                        self.r.time, 'monotonic', side_effect=[0, 0, 61]), patch.object(self.r.time, 'sleep'):
                    with self.assertRaisesRegex(self.r.RecoveryFailure, 'account history readiness failed'):
                        self.r.ready('/fixture/browser', verify_history=True)
                http.open.assert_called_once_with('http://127.0.0.1:8787/api/ready', timeout=1)

    def test_matching_process_and_history_allow_return(self):
        http = self.probe(b'{"ready": true}')
        with patch.object(self.r.urllib.request, 'build_opener', return_value=http):
            self.assertEqual(self.r.ready('/fixture/browser', verify_history=True), 123)

    def test_known_guard_reports_reason_without_logging_arbitrary_exception_text(self):
        known = self.r.failure_details(self.r.RecoveryFailure('pool configuration was not restored'))
        self.assertEqual(known['reason'], 'pool configuration was not restored')
        unknown = self.r.failure_details(ValueError('private-session-cookie'))
        self.assertEqual(unknown, {'error_type': 'ValueError'})
