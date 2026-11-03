import json
from contextlib import asynccontextmanager
import os
from pathlib import Path
import sys
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_host import body_capture as capture
from framerize_host.app import Host


class BodyCaptureTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.writer = capture.BodyCapture(self.root)
        self.path = '/ai/v3/messages/'

    def enable(self):
        marker = self.root / 'enabled'
        marker.touch(mode=0o600)
        return marker

    def records(self):
        self.writer.pending.join()
        return [json.loads(path.read_text()) for path in
                sorted(self.root.glob('capture-*.json'))]

    def test_default_off_and_unrelated_routes_do_not_capture(self):
        self.writer.record(self.path, 'job', 'request', {'text': 'private'})
        self.assertIsNone(self.writer.thread)
        self.enable()
        self.writer.record('/auth/ai/access-token', 'job', 'request', {})
        self.assertIsNone(self.writer.thread)

    def test_private_marker_expires_and_symlinks_are_rejected(self):
        marker = self.enable()
        self.assertTrue(self.writer.enabled())
        os.utime(marker, (time.time() - 7201,) * 2)
        self.assertFalse(self.writer.enabled())
        marker.touch()
        marker.chmod(0o644)
        self.assertFalse(self.writer.enabled())
        marker.rename(self.root / 'original')
        marker.symlink_to(self.root / 'original')
        self.assertFalse(self.writer.enabled())

    def test_bodies_preserve_thinking_but_remove_authentication(self):
        self.enable()
        body = {'thinking': {'type': 'adaptive'}, 'signature': 'actual-signed-thinking',
                'castleToken': 'private-castle', 'authorization': 'private-auth',
                'nested': [{'text': 'Bearer private-bearer password=private-password'}]}
        self.writer.record(self.path, 'job', 'request', body)
        result = self.records()[0]
        self.assertEqual(result['body']['signature'], 'actual-signed-thinking')
        self.assertEqual(body['castleToken'], 'private-castle')
        self.assertNotIn('private-', json.dumps(result))
        for path in self.root.glob('capture-*.json'):
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)

    def test_response_json_is_redacted_and_failure_is_not_a_response(self):
        self.enable()
        self.writer.response(self.path, 'one', {'status': 200,
            'body': json.dumps({'text': 'actual reply', 'token': 'private-token'})})
        self.writer.response(self.path, 'two', {'status': -1,
            'network_status': 504, 'body': 'NetworkError'})
        records = self.records()
        self.assertEqual(records[0]['body']['body']['text'], 'actual reply')
        self.assertNotIn('private-token', repr(records))
        self.assertFalse(records[1]['body']['complete_response'])

    def test_oversized_body_records_the_omission(self):
        self.enable()
        with patch.object(capture, 'RECORD_LIMIT', 256):
            self.writer.record(self.path, 'job', 'request', {'text': 'x' * 1024})
        record = self.records()[0]
        self.assertIn('body_omitted', record)
        self.assertNotIn('body', record)

    def test_disk_retention_is_bounded_under_repeated_requests(self):
        data = json.dumps({'text': 'x' * 200}).encode()
        with patch.multiple(capture, TOTAL_LIMIT=1024, FILE_LIMIT=4):
            for _ in range(64):
                self.writer._write(data)
        files = list(self.root.glob('capture-*.json'))
        self.assertLessEqual(len(files), 4)
        self.assertLessEqual(sum(path.stat().st_size for path in files), 1024)

    def test_full_queue_drops_diagnostics_without_waiting(self):
        self.enable()
        self.writer.thread = object()
        with patch.object(capture, 'emit') as emitted:
            for _ in range(10):
                self.writer.record(self.path, 'job', 'request', {})
        self.assertEqual(self.writer.pending.qsize(), 2)
        self.assertEqual(emitted.call_count, 1)

    def test_write_failure_is_contained(self):
        self.enable()
        with patch.object(self.writer, '_write', side_effect=OSError('private-detail')):
            with patch.object(capture, 'emit') as emitted:
                self.writer.record(self.path, 'job', 'request', {})
                self.writer.pending.join()
        self.assertNotIn('private-detail', repr(emitted.call_args_list))
        self.assertEqual(emitted.call_count, 1)

    def test_symlink_directory_and_nonprivate_directory_are_not_written(self):
        link = self.root / 'link'
        link.symlink_to(self.root)
        with self.assertRaises(OSError):
            capture.BodyCapture(link)._write(b'{}')
        self.root.chmod(0o755)
        with self.assertRaises(PermissionError):
            self.writer._write(b'{}')
        self.root.chmod(0o700)


class CaptureDispatchTests(unittest.IsolatedAsyncioTestCase):
    async def test_capture_observes_the_real_dispatch_without_credentials(self):
        @asynccontextmanager
        async def activity():
            yield
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'enabled').touch(mode=0o600)
            writer = capture.BodyCapture(root)
            result = dict(status=-1, network_status=504, body='NetworkError')
            host = SimpleNamespace(body_capture=writer,
                _account_for_page=lambda page: 'owner', _activity=lambda account: activity(),
                _run_job_active=AsyncMock(return_value=result))
            payload = {'request': {'model': 'anthropic/claude-fable-5.1',
                                   'max_tokens': 64000, 'thinking': {'type': 'adaptive'}}}
            returned = await Host._run_job(host, object(), '/ai/v3/messages/',
                payload, 'private-token', 'private-bearer', 'job', 'device', 180)
            self.assertIs(returned, result)
            writer.pending.join()
            records = [json.loads(path.read_text()) for path in
                       sorted(root.glob('capture-*.json'))]
            self.assertEqual(records[0]['body'], payload)
            self.assertEqual(records[1]['body']['network_status'], 504)
            self.assertFalse(records[1]['body']['complete_response'])
            self.assertNotIn('private-', repr(records))
            self.assertEqual(host._run_job_active.await_count, 1)
