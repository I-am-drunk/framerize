"""Inert checks for the one-shot transition controller."""
import importlib.util
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import Mock, patch


class TransitionTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        spec = importlib.util.spec_from_file_location('transition', Path(__file__).with_name('transition.py'))
        self.t = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.t)
        self.t.r.HERE = self.root
        self.current = {'MainPID': '123', 'ActiveState': 'active', 'InvocationID': 'original'}
        self.plan = dict(expected={'service': dict(self.current)}, native='/synthetic/native',
            profiles=str(self.root / 'profiles'), dropin=str(self.root / 'native.conf'),
            return_dropin=str(self.root / 'return.conf'), artifacts={})
        self.saved = {'eligible_accounts': 'all', 'active_capacity': 2}
        self.pool = dict(self.saved)
        self.slots, self.posts = [], []
        self.close_on_fence = True
        self.native = self.patch(self.t, 'native_http', side_effect=AssertionError('unexpected HTTP'))
        self.command = self.patch(self.t.r, 'command', side_effect=AssertionError('unexpected systemctl'))
        self.patch(self.t.r, 'status', side_effect=lambda: dict(self.current))
        self.patch(self.t.r, 'http_json', side_effect=self.http)
        self.patch(self.t.r, 'executable', return_value='/synthetic/native')
        self.patch(self.t, 'report')
        self.patch(self.t.time, 'monotonic', return_value=1000.0)

    def patch(self, target, name, **kwargs):
        mocked = patch.object(target, name, **kwargs)
        result = mocked.start()
        self.addCleanup(mocked.stop)
        return result

    def http(self, path, value=None):
        if path == '/api/pool':
            return {'residency': {'occupied': len(self.slots), 'residents': list(self.slots)}}
        self.assertEqual(path, '/api/pool/config')
        if value is not None:
            self.posts.append(value)
            self.pool = value
            if self.close_on_fence:
                self.slots = []
        return self.pool

    def request(self):
        path = self.root / 'request.json'
        path.write_text('{"fixture":true}')
        self.plan['artifacts']['request.json'] = self.t.r.digest(path)

    def test_active_users_expire_without_fencing_or_stopping(self):
        self.slots = [{'users': 1, 'close_started': False}]
        with self.assertRaisesRegex(RuntimeError, 'active work did not drain'):
            self.t.fence(self.plan, self.saved, seconds=0)
        self.assertEqual(self.posts, [])
        self.command.assert_not_called()

    def test_fence_requires_confirmed_empty_residency(self):
        self.slots = [{'users': 0, 'close_started': False, 'uncertain': True}]
        self.close_on_fence = False
        with self.assertRaisesRegex(RuntimeError, 'cleanup is unconfirmed'):
            self.t.fence(self.plan, self.saved)
        self.command.assert_not_called()

    def test_idle_residents_close_before_fence_is_reported(self):
        self.slots = [{'users': 0, 'close_started': False, 'uncertain': True}]
        self.t.fence(self.plan, self.saved)
        self.assertEqual(self.posts, [dict(self.saved, eligible_accounts=[])])
        self.assertEqual(self.slots, [])
        self.t.report.assert_called_with('fenced')

    def test_changed_original_owner_cannot_fence(self):
        self.current['InvocationID'] = 'different'
        with self.assertRaisesRegex(RuntimeError, 'original service changed'):
            self.t.fence(self.plan, self.saved)
        self.assertEqual(self.posts, [])
        self.command.assert_not_called()

    def test_lost_fence_response_cannot_start_a_service(self):
        def reject(path, value=None):
            if value is not None:
                raise TimeoutError('lost fence reply')
            return self.http(path)
        self.t.r.http_json.side_effect = reject
        with self.assertRaises(TimeoutError):
            self.t.fence(self.plan, self.saved)
        self.command.assert_not_called()

    def test_switch_refuses_missing_fence(self):
        with self.assertRaisesRegex(RuntimeError, 'pool fence changed'):
            self.t.switch(self.plan, self.saved)
        self.command.assert_not_called()
        self.assertFalse(Path(self.plan['dropin']).exists())

    def test_native_owner_change_is_not_treated_as_ready(self):
        self.t.r.executable.return_value = '/another/program'
        with self.assertRaisesRegex(RuntimeError, 'native service owner changed'):
            self.t.native_ready(self.plan)
        self.native.assert_not_called()

    def test_existing_attempt_never_reissues_request(self):
        self.request()
        self.t.claim_attempt()
        with self.assertRaises(FileExistsError):
            self.t.send_once(self.plan, self.current)
        self.native.assert_not_called()

    def test_lost_response_keeps_attempt_claimed(self):
        self.request()
        self.native.side_effect = TimeoutError('lost response')
        with self.assertRaises(TimeoutError):
            self.t.send_once(self.plan, self.current)
        with self.assertRaises(FileExistsError):
            self.t.send_once(self.plan, self.current)
        self.assertEqual(self.native.call_count, 1)

    def test_incomplete_body_is_not_reported_as_success(self):
        self.request()
        self.native.side_effect = None
        self.native.return_value = (200, {}, b'{"status":"incomplete","output":[]}')
        result = self.t.send_once(self.plan, self.current)
        self.assertFalse(result['complete'])
        self.assertFalse(result['text_is_ok'])

    def test_completed_canary_is_recorded_with_one_send(self):
        self.request()
        body = {'status': 'completed', 'output': [{'content': [
            {'type': 'output_text', 'text': 'OK'}]}]}
        self.native.side_effect = None
        self.native.return_value = (200, {}, json.dumps(body).encode())
        result = self.t.send_once(self.plan, self.current)
        self.assertTrue(result['complete'] and result['text_is_ok'])
        self.assertEqual(self.native.call_count, 1)
        self.assertEqual(json.loads((self.root / 'private-response.json').read_bytes()), body)

    def history(self, **changes):
        source = dict(boot_id='boot', unit='fixture.service', cgroup='/user.slice/fixture.service', invocation='old')
        (self.root / 'source-runtime.json').write_text(json.dumps(source))
        record = dict(kind='legacy-expiry', source_runtime=source,
            successor_runtime=dict(source, invocation='new'), started_at=100.0,
            completed_at=700.0, window_seconds=600.0, reserve_blocked_until=86500.0)
        record.update(changes)
        path = Path(self.plan['profiles']) / '.sidecar/camoufox/rate-history.sqlite3'
        path.parent.mkdir(parents=True, exist_ok=True)
        path.unlink(missing_ok=True)
        db = sqlite3.connect(path)
        try:
            with db:
                db.execute('CREATE TABLE migration_evidence(record TEXT)')
                db.execute('INSERT INTO migration_evidence VALUES (?)', (json.dumps(record),))
                db.execute('CREATE TABLE accounts(id INTEGER)')
                db.execute('INSERT INTO accounts VALUES (1)')
        finally:
            db.close()

    def test_new_same_service_expiry_is_accepted(self):
        self.history()
        result = self.t.verify_history(self.plan, 99, {'InvocationID': 'new'})
        self.assertEqual(result, {'accounts': 1, 'observed_seconds': 600})

    def test_short_old_or_unreserved_history_is_rejected(self):
        for changes in ({'completed_at': 699}, {'started_at': 98}, {'reserve_blocked_until': 0}):
            with self.subTest(changes=changes):
                self.history(**changes)
                with self.assertRaises(RuntimeError):
                    self.t.verify_history(self.plan, 99, {'InvocationID': 'new'})

    def test_different_successor_cannot_reuse_migration_evidence(self):
        self.history()
        with self.assertRaisesRegex(RuntimeError, 'migration owner differs'):
            self.t.verify_history(self.plan, 99, {'InvocationID': 'other'})

    def test_history_failure_prevents_a_canary_send(self):
        self.patch(self.t.r, 'read_plan', return_value=self.plan)
        self.patch(self.t, 'supervised')
        self.patch(self.t, 'preflight', return_value=self.saved)
        self.patch(self.t, 'fence')
        self.patch(self.t, 'switch', return_value=99)
        self.patch(self.t, 'native_ready', return_value=self.current)
        self.patch(self.t, 'verify_history', side_effect=RuntimeError('unconfirmed'))
        send = self.patch(self.t, 'send_once')
        with self.assertRaisesRegex(RuntimeError, 'unconfirmed'):
            self.t.execute()
        send.assert_not_called()

    def test_failure_still_runs_recovery(self):
        self.patch(self.t, 'execute', side_effect=RuntimeError('fixture failure'))
        recover = self.patch(self.t.r, 'restore', return_value={'status': 'restored-original'})
        self.assertEqual(self.t.main(self.root), 1)
        recover.assert_called_once_with()

    def test_success_cannot_hide_failed_recovery(self):
        self.patch(self.t, 'execute', return_value=0)
        self.patch(self.t.r, 'restore', side_effect=RuntimeError('cleanup failed'))
        self.assertEqual(self.t.main(self.root), 2)

    def test_unowned_supervisor_is_refused(self):
        self.plan['supervisor_unit'] = 'fixture.service'
        self.command.side_effect = None
        self.command.return_value = 'MainPID=0\nInvocationID=not-ours\n'
        with self.assertRaisesRegex(RuntimeError, 'supervisor differs'):
            self.t.supervised(self.plan)
        self.assertEqual(self.posts, [])


if __name__ == '__main__':
    unittest.main()
