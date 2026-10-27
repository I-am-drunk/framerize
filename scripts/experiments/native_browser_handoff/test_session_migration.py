import contextlib
import fcntl
import hashlib
import io
import json
import os
from pathlib import Path
import signal
import sqlite3
import stat
import tempfile
import unittest
from unittest import mock
import session_migration as migration


class MigrationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='framerize-session-migration-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.sessions = self.root / 'sessions'
        self.sessions.mkdir(mode=0o700)
        self.lock = self.sessions / '.owner.lock'
        self.lock.touch(mode=0o600)
        self.user = 'Exact-User-é'
        self.cookie = b'session=synthetic-only; other=unchanged'
        self.legacy = self.sessions / (self.user + '.session')
        self.legacy.write_bytes(self.cookie)
        self.legacy.chmod(0o600)
        self.destination = self.sessions / migration.destination(self.user)
        self.database = self.root / 'accounts.db'
        self.account = {'id': '00000000-0000-0000-0000-000000000001',
                        'framer_user_id': self.user, 'proxy': None, 'state': 'ready'}
        self.raw = json.dumps(self.account)
        with sqlite3.connect(self.database) as db:
            db.execute('CREATE TABLE accounts (id TEXT PRIMARY KEY, json TEXT)')
            db.execute('INSERT INTO accounts VALUES (?,?)', (self.account['id'], self.raw))
        self.plan = {'version': 1, 'account_id': self.account['id'],
                     'expected_user_id': self.user, 'database': str(self.database),
                     'session_directory': str(self.sessions),
                     'account_sha256': hashlib.sha256(self.raw.encode()).hexdigest()}
        self.verify = mock.Mock(return_value=self.user)

    def assert_unmodified(self):
        self.assertEqual(self.legacy.read_bytes(), self.cookie)
        self.assertFalse(self.destination.exists())
        self.assertEqual(list(self.sessions.glob('.session-migration-*.tmp')), [])

    def assert_lock_available(self):
        with self.lock.open('r+b') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)

    def test_success_preserves_legacy_account_and_exact_cookie(self):
        result = migration.migrate(self.plan, self.verify)
        self.verify.assert_called_once_with(self.cookie)
        expected = b'FRMSESS2' + len(self.user.encode()).to_bytes(8, 'big') + self.user.encode() + self.cookie
        self.assertEqual(self.destination.read_bytes(), expected)
        self.assertEqual(self.destination.stat().st_mode & 0o777, 0o600)
        self.assertEqual(self.destination.stat().st_nlink, 1)
        self.assertEqual(self.legacy.read_bytes(), self.cookie)
        with sqlite3.connect(self.database) as db:
            self.assertEqual(db.execute('SELECT json FROM accounts').fetchone()[0], self.raw)
        self.assertEqual(result['status'], 'V2_SESSION_PUBLISHED_IDENTITY_VERIFIED')
        self.assertTrue(result['admission_still_required'])
        self.assert_lock_available()

    def test_mismatch_and_verifier_failure_never_publish(self):
        for value in (self.user.lower(), '', None, {'id': self.user}):
            with self.subTest(value_type=type(value).__name__):
                with self.assertRaisesRegex(migration.Refused, 'AUTHENTICATED_USER_MISMATCH'):
                    migration.migrate(self.plan, lambda cookie: value)
                self.assert_unmodified()
        with self.assertRaisesRegex(migration.Refused, 'IDENTITY_HTTP_REFUSED'):
            migration.migrate(self.plan, mock.Mock(side_effect=migration.Refused('IDENTITY_HTTP_REFUSED')))
        self.assert_unmodified()
        self.assert_lock_available()

    def test_existing_destination_refused_before_cookie_and_network(self):
        creators = [lambda: self.destination.write_bytes(b'existing'),
                    lambda: self.destination.mkdir(),
                    lambda: self.destination.symlink_to(self.legacy),
                    lambda: os.mkfifo(self.destination, 0o600)]
        for create in creators:
            create()
            with mock.patch.object(migration, 'read_cookie', side_effect=AssertionError('must not read')):
                with self.assertRaisesRegex(migration.Refused, 'V2_RECORD_ALREADY_EXISTS'):
                    migration.migrate(self.plan, self.verify)
            if self.destination.is_dir():
                self.destination.rmdir()
            else:
                self.destination.unlink()
        self.verify.assert_not_called()
        self.assert_unmodified()

    def test_active_owner_refused_before_cookie_read(self):
        with self.lock.open('r+b') as held:
            fcntl.flock(held, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaisesRegex(migration.Refused, 'SESSION_OWNER_BUSY'):
                migration.migrate(self.plan, self.verify)
        self.verify.assert_not_called()

    def test_nonregular_public_and_invalid_legacy_refused(self):
        self.legacy.unlink()
        os.mkfifo(self.legacy, 0o600)
        with self.assertRaises(migration.Refused):
            migration.migrate(self.plan, self.verify)
        self.legacy.unlink()
        self.legacy.symlink_to(self.lock)
        with self.assertRaises(OSError):
            migration.migrate(self.plan, self.verify)
        self.legacy.unlink()
        self.legacy.write_bytes(self.cookie)
        self.legacy.chmod(0o644)
        with self.assertRaises(migration.Refused):
            migration.migrate(self.plan, self.verify)
        self.legacy.chmod(0o600)
        for value in (b'', b'x' * 16385, b'session=bad\r\nHeader: value', b'\xff'):
            self.legacy.write_bytes(value)
            with self.assertRaises(migration.Refused):
                migration.migrate(self.plan, self.verify)
        self.verify.assert_not_called()
        self.assertFalse(self.destination.exists())

    def test_account_changes_and_proxy_refused(self):
        for changes in ({'id': '00000000-0000-0000-0000-000000000002'},
                        {'framer_user_id': self.user.lower()}, {'state': 'changed'},
                        {'proxy': ''}, {'proxy': 'https://proxy.invalid'}):
            altered = json.dumps(dict(self.account, **changes))
            with sqlite3.connect(self.database) as db:
                db.execute('UPDATE accounts SET json=?', (altered,))
            plan = dict(self.plan)
            if 'proxy' in changes:
                plan['account_sha256'] = hashlib.sha256(altered.encode()).hexdigest()
            with self.assertRaises(migration.Refused):
                migration.migrate(plan, self.verify)
        self.verify.assert_not_called()

    def test_cookie_change_during_verification_refused(self):
        def verify(cookie):
            self.legacy.write_bytes(b'session=new-synthetic-value')
            return self.user
        with self.assertRaisesRegex(migration.Refused, 'SOURCE_BINDING_CHANGED'):
            migration.migrate(self.plan, verify)
        self.assertFalse(self.destination.exists())
        self.assertEqual(list(self.sessions.glob('.session-migration-*.tmp')), [])
        self.assert_lock_available()

    def test_account_change_during_verification_refused(self):
        def verify(cookie):
            with sqlite3.connect(self.database) as db:
                db.execute('UPDATE accounts SET json=?', (json.dumps(dict(self.account, state='changed')),))
            return self.user
        with self.assertRaisesRegex(migration.Refused, 'ACCOUNT_BINDING_CHANGED'):
            migration.migrate(self.plan, verify)
        self.assert_unmodified()

    def test_owner_file_replacement_refused(self):
        def verify(cookie):
            self.lock.unlink()
            self.lock.touch(mode=0o600)
            return self.user
        with self.assertRaisesRegex(migration.Refused, 'SOURCE_BINDING_CHANGED'):
            migration.migrate(self.plan, verify)
        self.assert_unmodified()

    def test_destination_race_never_replaces_other_file(self):
        real_link = os.link
        def race(source, target, **kwargs):
            self.destination.write_bytes(b'concurrent-record')
            return real_link(source, target, **kwargs)
        with mock.patch.object(migration.os, 'link', side_effect=race):
            with self.assertRaises(FileExistsError):
                migration.migrate(self.plan, self.verify)
        self.assertEqual(self.destination.read_bytes(), b'concurrent-record')
        self.assertEqual(self.legacy.read_bytes(), self.cookie)
        self.assertEqual(list(self.sessions.glob('.session-migration-*.tmp')), [])

    def test_prepublication_failure_removes_only_owned_temporary(self):
        for operation in ('write', 'fsync'):
            with self.subTest(operation=operation):
                with mock.patch.object(migration.os, operation, side_effect=OSError('synthetic')):
                    with self.assertRaises(OSError):
                        migration.migrate(self.plan, self.verify)
                self.assert_unmodified()
                self.assert_lock_available()
        with self.assertRaises(KeyboardInterrupt):
            migration.migrate(self.plan, mock.Mock(side_effect=KeyboardInterrupt))
        self.assert_unmodified()

    def test_temporary_collision_preserves_existing_file(self):
        path = self.sessions / '.session-migration-fixed.tmp'
        path.write_bytes(b'not-ours')
        with mock.patch.object(migration.secrets, 'token_hex', return_value='fixed'):
            with self.assertRaises(FileExistsError):
                migration.migrate(self.plan, self.verify)
        self.assertEqual(path.read_bytes(), b'not-ours')
        self.assertFalse(self.destination.exists())

    def test_directory_sync_failure_keeps_published_record_and_refuses_retry(self):
        real_sync = os.fsync
        def sync(fd):
            if stat.S_ISDIR(os.fstat(fd).st_mode):
                raise OSError('synthetic directory sync failure')
            return real_sync(fd)
        with mock.patch.object(migration.os, 'fsync', side_effect=sync):
            with self.assertRaisesRegex(migration.Refused, 'V2_PUBLISHED_DURABILITY_UNCONFIRMED'):
                migration.migrate(self.plan, self.verify)
        self.assertTrue(self.destination.read_bytes().endswith(self.cookie))
        self.assertEqual(self.legacy.read_bytes(), self.cookie)
        self.verify.reset_mock()
        with self.assertRaisesRegex(migration.Refused, 'V2_RECORD_ALREADY_EXISTS'):
            migration.migrate(self.plan, self.verify)
        self.verify.assert_not_called()
        self.assert_lock_available()

    def test_dry_run_never_opens_sessions_or_calls_network(self):
        plan_path = self.root / 'plan.json'
        plan_path.write_text(json.dumps(self.plan))
        plan_path.chmod(0o600)
        with mock.patch.object(migration, 'migrate', side_effect=AssertionError('must not run')):
            with contextlib.redirect_stdout(io.StringIO()) as output:
                self.assertEqual(migration.main(['--plan', str(plan_path)]), 0)
        self.assertEqual(json.loads(output.getvalue())['status'], 'PLAN_VALID_NO_EXECUTION')
        self.assert_unmodified()

    def test_plan_requires_private_file_and_exact_fields(self):
        plan_path = self.root / 'plan.json'
        plan_path.write_text(json.dumps(self.plan))
        plan_path.chmod(0o644)
        with self.assertRaises(migration.Refused):
            migration.read_plan(plan_path)
        with self.assertRaises(migration.Refused):
            migration.validate_plan(dict(self.plan, endpoint='https://unexpected.invalid'))

    def test_fixed_identity_request_preserves_exact_cookie_and_closes(self):
        connection = mock.Mock()
        response = connection.getresponse.return_value
        response.status = 200
        response.read.return_value = json.dumps({'id': self.user}).encode()
        with mock.patch.object(migration.http.client, 'HTTPSConnection', return_value=connection) as factory:
            self.assertEqual(migration.verify_identity(self.cookie), self.user)
        self.assertEqual(factory.call_args.args, ('api.framer.com',))
        self.assertEqual(factory.call_args.kwargs['timeout'], 10)
        self.assertEqual(connection.request.call_args.args, ('GET', '/web/users/me'))
        self.assertEqual(connection.request.call_args.kwargs['headers']['Cookie'], self.cookie.decode())
        response.read.assert_called_once_with(65537)
        connection.close.assert_called_once()
        self.assertEqual(signal.getitimer(signal.ITIMER_REAL), (0.0, 0.0))

    def test_identity_redirects_and_failures_have_no_followup(self):
        for status in (301, 302, 401, 403, 429, 500):
            connection = mock.Mock()
            connection.getresponse.return_value.status = status
            with mock.patch.object(migration.http.client, 'HTTPSConnection', return_value=connection):
                with self.assertRaisesRegex(migration.Refused, 'IDENTITY_HTTP_REFUSED'):
                    migration.verify_identity(self.cookie)
            connection.request.assert_called_once()
            connection.getresponse.return_value.read.assert_not_called()
            connection.close.assert_called_once()

    def test_identity_response_limits_and_ambiguity(self):
        bodies = [b'x' * 65537, b'not-json', b'\xff', b'[]', b'{}',
                  b'{"id":1}', b'{"id":"one","id":"two"}']
        for body in bodies:
            connection = mock.Mock()
            connection.getresponse.return_value.status = 200
            connection.getresponse.return_value.read.return_value = body
            with mock.patch.object(migration.http.client, 'HTTPSConnection', return_value=connection):
                with self.assertRaises(migration.Refused):
                    migration.verify_identity(self.cookie)
            connection.close.assert_called_once()

    def test_identity_deadline_restores_handler_and_closes(self):
        prior = signal.getsignal(signal.SIGALRM)
        connection = mock.Mock()
        connection.request.side_effect = lambda *args, **kwargs: os.kill(os.getpid(), signal.SIGALRM)
        with mock.patch.object(migration.http.client, 'HTTPSConnection', return_value=connection):
            with self.assertRaisesRegex(migration.Refused, 'IDENTITY_REQUEST_TIMEOUT'):
                migration.verify_identity(self.cookie)
        connection.close.assert_called_once()
        self.assertEqual(signal.getsignal(signal.SIGALRM), prior)
        self.assertEqual(signal.getitimer(signal.ITIMER_REAL), (0.0, 0.0))

    def test_cli_diagnostics_never_emit_exception_payload(self):
        plan_path = self.root / 'plan.json'
        plan_path.write_text(json.dumps(self.plan))
        plan_path.chmod(0o600)
        with mock.patch.object(migration, 'migrate', side_effect=ValueError('session=private-value')):
            with contextlib.redirect_stdout(io.StringIO()) as output:
                self.assertEqual(migration.main(['--plan', str(plan_path), '--execute']), 2)
        self.assertNotIn('private-value', output.getvalue())
        self.assertEqual(json.loads(output.getvalue())['reason'], 'MIGRATION_IO_OR_DECODE_FAILED')

    def test_names_match_published_store_vectors_without_normalizing(self):
        vectors = {
            'A': '4fb4ad61e72a2a1d37ae3685ec6cbea60f83eadec56b3776185cbb4af2a36cb4',
            'a': 'b606109a223091877795db3eb4f6d20ac0ad9da36026625c8eec736b682838cf',
            'é': '0d46d4cf021ad6ad439b4ae88a2671e00a1b85dbf916e048f0eac03e064ea8df',
            'e\u0301': 'a5c9128f10f8411fceaa53d4e9000b4eca0837318a11fd2189cecf7e0200f5cf'}
        for user, digest in vectors.items():
            self.assertEqual(migration.destination(user), 'session-v2-' + digest + '.data')

    def test_existing_timer_is_not_overwritten(self):
        signal.setitimer(signal.ITIMER_REAL, 60)
        try:
            connection = mock.Mock()
            with mock.patch.object(migration.http.client, 'HTTPSConnection', return_value=connection):
                with self.assertRaisesRegex(migration.Refused, 'EXISTING_PROCESS_TIMER'):
                    migration.verify_identity(self.cookie)
            connection.request.assert_not_called()
            connection.close.assert_called_once()
            self.assertGreater(signal.getitimer(signal.ITIMER_REAL)[0], 0)
        finally:
            signal.setitimer(signal.ITIMER_REAL, 0)


if __name__ == '__main__':
    unittest.main(verbosity=2)
