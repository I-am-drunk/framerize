"""Release switching uses only temporary files and an inert service adapter."""
import fcntl
from contextlib import nullcontext
import io
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'scripts'))
import release
import release_activation as activation


class Service:
    def __init__(self):
        self.calls, self.fail = [], set()

    def validate(self, old, new):
        pass

    def stop(self, executable=None):
        self.calls.append('stop')

    def start(self, package):
        self.calls.append(package.name)
        if package.name in self.fail:
            raise RuntimeError('fixture startup failure')


class ActivationTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.installation, self.profiles = self.root / 'install', self.root / 'profiles'
        self.old, self.new = self.package('old'), self.package('new')
        self.current = self.installation / 'current'
        self.current.symlink_to(self.old)
        self.history = self.profiles / '.sidecar/camoufox/rate-history.sqlite3'
        self.history.parent.mkdir(parents=True)
        with sqlite3.connect(self.history) as db:
            db.execute('PRAGMA user_version=2')
            db.execute('CREATE TABLE unchanged (value TEXT)')
            db.execute("INSERT INTO unchanged VALUES ('preserve')")
        self.before = self.history.read_bytes()
        self.service = Service()
        self.receipt = self.root / 'qualification.json'
        checks = [dict(route=route, stream=stream, http_status=200,
            terminal=True, expected_content=True) for route in (
                '/openai/v1/responses', '/anthropic/v1/messages')
            for stream in (False, True)]
        evidence = self.root / 'inference-checks.json'
        evidence.write_text(json.dumps(checks))
        self.qualification = dict(format=1, application='framerize', commit=None,
            binary_sha256=release.sha256(self.new / 'bin/framerize'),
            manifest_sha256=release.sha256(self.new / 'manifest.json'),
            engine='camoufox', verified_at=time.time(), checks=checks,
            isolated=dict(port=8792, account_outside_production=True,
                workspace_outside_production=True, session_lock_owned=True,
                shared_token_socket=False, paid_routes_enabled=False),
            evidence_sha256={evidence.name: release.sha256(evidence)})
        self.receipt.write_text(json.dumps(self.qualification))

    def package(self, name):
        root = self.installation / 'releases' / name
        for path in ('bin/framerize', 'runtime/host.py', 'runtime/runtime-manifest.json',
                     'web/index.html', 'share/systemd/user/framerize.service'):
            target = root / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text('fixture')
        (root / 'bin/framerize').chmod(0o755)
        (root / 'build-info.json').write_text(json.dumps(dict(format=1,
            application='framerize', development=False, source_dirty=False,
            build_profile='release', rate_history_schema=2, activation_protocol=1, platform='fixture',
            checkpoint_history=dict(read_versions=[1], write_version=1))))
        self.manifest(root)
        return root

    @staticmethod
    def manifest(root):
        files = {path.relative_to(root).as_posix(): release.sha256(path)
                 for path in root.rglob('*') if path.is_file() and path.name != 'manifest.json'}
        (root / 'manifest.json').write_text(json.dumps(files))

    def activate(self):
        return activation.activate(self.new, self.installation, self.profiles,
                                   self.service, qualification=self.receipt)

    def checkpoint_package(self, root, readers, writer):
        path = root / 'build-info.json'
        info = json.loads(path.read_text())
        info['checkpoint_history'] = dict(read_versions=readers, write_version=writer)
        path.write_text(json.dumps(info))
        self.manifest(root)
        if root == self.new:
            self.qualification['manifest_sha256'] = release.sha256(root / 'manifest.json')
            self.receipt.write_text(json.dumps(self.qualification))

    def checkpoint_database(self, version):
        path = self.profiles / '.checkpoints/history.sqlite3'
        path.parent.mkdir(exist_ok=True)
        with sqlite3.connect(path) as db:
            db.execute('CREATE TABLE checkpoints (fingerprint TEXT PRIMARY KEY, layout TEXT)')
            db.execute('INSERT INTO checkpoints VALUES (?, ?)',
                       ('fixture', json.dumps(dict(version=version))))
        return path

    def test_checkpoint_upgrade_requires_a_compatible_rollback_reader(self):
        self.checkpoint_package(self.new, [1, 2], 2)
        with self.assertRaisesRegex(ValueError, 'checkpoint.*rollback'):
            self.activate()
        self.assertEqual(self.service.calls, [])
        self.assertEqual(self.current.resolve(), self.old)

    def test_checkpoint_reader_upgrade_keeps_old_writer_rollback_safe(self):
        self.checkpoint_package(self.new, [1, 2], 1)
        history = self.checkpoint_database(1)
        before = history.read_bytes()
        self.assertEqual(self.activate()['status'], 'activated')
        self.assertEqual(history.read_bytes(), before)

    def test_checkpoint_metadata_is_validated_before_service_actions(self):
        for readers, writer in (([1, 2], 3), ([True], 1), ([1], True),
                                ([1, 1], 1), ([2], 2), ([], 1), ('1', 1)):
            with self.subTest(readers=readers, writer=writer):
                self.checkpoint_package(self.new, readers, writer)
                with self.assertRaisesRegex(ValueError, 'checkpoint history package metadata'):
                    self.activate()
                self.assertEqual(self.service.calls, [])

    def test_checkpoint_candidate_must_declare_its_format(self):
        path = self.new / 'build-info.json'
        info = json.loads(path.read_text())
        del info['checkpoint_history']
        path.write_text(json.dumps(info))
        self.manifest(self.new)
        with self.assertRaisesRegex(ValueError, 'checkpoint history package metadata'):
            self.activate()
        self.assertEqual(self.service.calls, [])

    def test_legacy_checkpoint_package_can_upgrade_to_a_compatible_reader(self):
        path = self.old / 'build-info.json'
        info = json.loads(path.read_text())
        del info['checkpoint_history']
        path.write_text(json.dumps(info))
        self.manifest(self.old)
        self.checkpoint_package(self.new, [1, 2], 1)
        self.assertEqual(self.activate()['status'], 'activated')

    def test_checkpoint_downgrade_rejects_a_reader_missing_the_old_writer(self):
        self.checkpoint_package(self.old, [1, 2], 2)
        with self.assertRaisesRegex(ValueError, 'checkpoint.*rollback'):
            self.activate()
        self.assertEqual(self.service.calls, [])

    def test_invalid_stored_checkpoint_version_never_stops_the_service(self):
        history = self.checkpoint_database(True)
        before = history.read_bytes()
        with self.assertRaisesRegex(ValueError, 'stored checkpoint version'):
            self.activate()
        self.assertEqual(self.service.calls, [])
        self.assertEqual(history.read_bytes(), before)

    def test_malformed_checkpoint_storage_never_stops_the_service(self):
        history = self.checkpoint_database(1)
        with sqlite3.connect(history) as db:
            db.execute("UPDATE checkpoints SET layout='invalid json'")
        before = history.read_bytes()
        with self.assertRaisesRegex(ValueError, 'checkpoint history could not be inspected'):
            self.activate()
        self.assertEqual(self.service.calls, [])
        self.assertEqual(history.read_bytes(), before)

    def test_checkpoint_symlink_is_not_followed_during_activation(self):
        target = self.root / 'unrelated'
        target.mkdir()
        (self.profiles / '.checkpoints').symlink_to(target, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'checkpoint history must not use symbolic links'):
            self.activate()
        self.assertEqual(self.service.calls, [])
        self.assertEqual(list(target.iterdir()), [])

    def test_existing_checkpoint_versions_must_be_readable_by_both_packages(self):
        history = self.checkpoint_database(2)
        before = history.read_bytes()
        with self.assertRaisesRegex(ValueError, 'checkpoint.*version'):
            self.activate()
        self.assertEqual(self.service.calls, [])
        self.assertEqual(history.read_bytes(), before)

    def test_failed_activation_keeps_new_history_readable_on_rollback(self):
        self.checkpoint_package(self.old, [1, 2], 1)
        self.checkpoint_package(self.new, [1, 2], 2)
        self.service.fail.add('new')
        original = self.service.start
        def start(package):
            if package == self.new:
                self.checkpoint_database(2)
            original(package)
        with patch.object(self.service, 'start', side_effect=start):
            with self.assertRaisesRegex(RuntimeError, 'previous release restored'):
                self.activate()
        self.assertEqual(self.current.resolve(), self.old)
        with sqlite3.connect(self.profiles / '.checkpoints/history.sqlite3') as db:
            self.assertEqual(json.loads(db.execute('SELECT layout FROM checkpoints').fetchone()[0])['version'], 2)

    def test_success_switches_only_the_package_and_preserves_history(self):
        self.assertEqual(self.activate()['status'], 'activated')
        self.assertEqual(self.current.resolve(), self.new)
        self.assertEqual(self.service.calls, ['stop', 'new'])
        self.assertEqual(self.history.read_bytes(), self.before)
        self.assertFalse((self.installation / 'update.json').exists())
        self.assertEqual(self.activate()['status'], 'unchanged')
        self.assertEqual(self.service.calls, ['stop', 'new', 'new'])

    def test_failed_start_restores_the_previous_package_without_rolling_back_data(self):
        self.service.fail.add('new')
        with self.assertRaisesRegex(RuntimeError, 'previous release restored'):
            self.activate()
        self.assertEqual(self.current.resolve(), self.old)
        self.assertEqual(self.service.calls, ['stop', 'new', 'stop', 'old'])
        self.assertEqual(self.history.read_bytes(), self.before)

    def test_failed_rollback_leaves_a_recoverable_record(self):
        self.service.fail.update(('old', 'new'))
        with self.assertRaisesRegex(RuntimeError, 'rollback failed'):
            self.activate()
        self.assertTrue((self.installation / 'update.json').is_file())
        self.service.fail.clear()
        self.assertEqual(self.activate()['status'], 'recovered')
        self.assertEqual(self.current.resolve(), self.old)
        self.assertEqual(self.service.calls[-2:], ['stop', 'old'])
        self.assertFalse((self.installation / 'update.json').exists())

    def test_interrupted_pointer_switch_restores_old_without_replaying_new(self):
        (self.installation / 'update.json').write_text(json.dumps(
            {'version': 1, 'old': str(self.old), 'new': str(self.new)}))
        self.current.unlink()
        self.current.symlink_to(self.new)
        self.assertEqual(self.activate()['status'], 'recovered')
        self.assertEqual(self.service.calls, ['stop', 'old'])
        self.assertEqual(self.current.resolve(), self.old)

    def test_missing_history_refuses_before_stopping_service(self):
        self.history.unlink()
        with self.assertRaisesRegex(ValueError, 'migration'):
            self.activate()
        self.assertEqual(self.service.calls, [])
        self.assertEqual(self.current.resolve(), self.old)

    def test_missing_inference_qualification_never_stops_the_service(self):
        with self.assertRaisesRegex(ValueError, 'qualification is required'):
            activation.activate(self.new, self.installation, self.profiles, self.service)
        self.assertEqual(self.service.calls, [])
        self.assertEqual(self.current.resolve(), self.old)

    def test_health_or_credit_refusal_cannot_qualify_a_release(self):
        for status in (402, 403, 500, 503):
            with self.subTest(status=status):
                self.qualification['checks'][0]['http_status'] = status
                self.receipt.write_text(json.dumps(self.qualification))
                with self.assertRaisesRegex(ValueError, 'delivery must all pass'):
                    self.activate()
                self.assertEqual(self.service.calls, [])
                self.assertEqual(self.current.resolve(), self.old)

    def test_truncated_success_cannot_qualify_a_release(self):
        self.qualification['checks'][0]['terminal'] = False
        self.receipt.write_text(json.dumps(self.qualification))
        with self.assertRaisesRegex(ValueError, 'delivery must all pass'):
            self.activate()
        self.assertEqual(self.service.calls, [])

    def test_stale_or_different_package_evidence_never_stops_service(self):
        original = json.loads(self.receipt.read_text())
        for field, value in (('binary_sha256', '0' * 64),
                             ('manifest_sha256', '0' * 64),
                             ('verified_at', time.time() - 86401)):
            self.receipt.write_text(json.dumps(dict(original, **{field: value})))
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.activate()
            self.assertEqual(self.service.calls, [])

    def test_shared_or_paid_test_configuration_never_stops_service(self):
        original = json.loads(self.receipt.read_text())
        for field, value in (('port', 8787), ('account_outside_production', False),
                             ('workspace_outside_production', False),
                             ('session_lock_owned', False),
                             ('shared_token_socket', True), ('paid_routes_enabled', True)):
            record = json.loads(json.dumps(original))
            record['isolated'][field] = value
            self.receipt.write_text(json.dumps(record))
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.activate()
            self.assertEqual(self.service.calls, [])

    def test_changed_evidence_never_stops_service(self):
        (self.root / 'inference-checks.json').write_text('changed')
        with self.assertRaisesRegex(ValueError, 'evidence changed'):
            self.activate()
        self.assertEqual(self.service.calls, [])

    def test_receipt_must_match_the_recorded_request_results(self):
        self.qualification['checks'][0]['seconds'] = 1.0
        self.receipt.write_text(json.dumps(self.qualification))
        with self.assertRaisesRegex(ValueError, 'claims differ'):
            self.activate()
        self.assertEqual(self.service.calls, [])

    def test_package_or_schema_mismatch_refuses_before_service_actions(self):
        path = self.new / 'build-info.json'
        original = json.loads(path.read_text())
        for field, value in (('development', True), ('rate_history_schema', 3),
                             ('platform', 'other'), ('application', 'framerize-native'),
                             ('activation_protocol', None)):
            with self.subTest(field=field):
                path.write_text(json.dumps(dict(original, **{field: value})))
                self.manifest(self.new)
                with self.assertRaises(ValueError):
                    self.activate()
                self.assertEqual(self.service.calls, [])
                self.assertEqual(self.current.resolve(), self.old)
        path.write_text(json.dumps(original))
        self.manifest(self.new)
        (self.new / 'web/index.html').write_text('unexpected edit')
        with self.assertRaisesRegex(ValueError, 'checksum'):
            self.activate()
        self.assertEqual(self.service.calls, [])

    def test_second_updater_cannot_queue_a_later_unintended_switch(self):
        with (self.installation / '.update.lock').open('a') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            with self.assertRaises(BlockingIOError):
                self.activate()
        self.assertEqual(self.service.calls, [])

    def test_readiness_rejects_the_wrong_process_even_with_http_200(self):
        service = activation.UserService()
        info = {'ActiveState': 'active', 'MainPID': '42'}
        responses = []
        for pid, body in (('99', b'ok'), ('42', b'ok'), ('42', b'{"ready":true}')):
            response = io.BytesIO(body)
            response.status = 200
            response.headers = {'x-framerize-pid': pid}
            responses.append(nullcontext(response))
        with patch.object(service, 'command') as command, \
             patch.object(service, 'show', return_value=info), \
             patch.object(service, 'executable', return_value=self.new / 'bin/framerize'), \
             patch.object(service.http, 'open', side_effect=responses) as opened, \
             patch.object(activation.time, 'sleep'):
            service.start(self.new)
        self.assertEqual(opened.call_count, 3)
        command.assert_called_once_with('start', '--', 'framerize.service')

    def test_start_waits_for_account_history_readiness(self):
        service = activation.UserService()
        info = {'ActiveState': 'active', 'MainPID': '42'}
        responses = []
        for body in (b'ok', b'{"ready":false}', b'ok', b'{"ready":true}'):
            response = io.BytesIO(body)
            response.status = 200
            response.headers = {'x-framerize-pid': '42'}
            responses.append(nullcontext(response))
        with patch.object(service, 'command') as command, \
             patch.object(service, 'show', return_value=info), \
             patch.object(service, 'executable', return_value=self.new / 'bin/framerize'), \
             patch.object(service.http, 'open', side_effect=responses) as opened, \
             patch.object(activation.time, 'sleep'):
            service.start(self.new)
        self.assertEqual(opened.call_count, 4)
        command.assert_called_once_with('start', '--', 'framerize.service')

    def test_activation_stop_uses_the_verified_graceful_drain(self):
        service = activation.UserService()
        state = dict(ActiveState='active', MainPID='42', ControlGroup='/fixture')
        expected = self.old / 'bin/framerize'
        with patch('release_drain.state', return_value=state), \
             patch('release_drain.stop', return_value={'clean_exit': True}) as drain:
            service.stop(expected)
        drain.assert_called_once_with(service, expected)

    def test_inactive_service_with_leftover_processes_cannot_be_replaced(self):
        service = activation.UserService()
        state = dict(ActiveState='failed', MainPID='0', ControlGroup='/fixture')
        with patch('release_drain.state', return_value=state), \
             patch('release_drain.descendants', return_value={42}):
            with self.assertRaisesRegex(RuntimeError, 'still has processes'):
                service.stop(self.old / 'bin/framerize')

    def test_service_restart_during_inactive_preflight_cannot_be_replaced(self):
        service = activation.UserService()
        stopped = dict(ActiveState='failed', MainPID='0', ControlGroup='/fixture')
        restarted = dict(ActiveState='active', MainPID='42', ControlGroup='/fixture')
        with patch('release_drain.state', side_effect=[stopped, restarted]), \
             patch('release_drain.descendants', return_value=set()):
            with self.assertRaisesRegex(RuntimeError, 'state changed'):
                service.stop(self.old / 'bin/framerize')

    def test_service_preflight_refuses_overrides_and_an_unrelated_process(self):
        service = activation.UserService()
        service.unit_file = self.root / 'framerize.service'
        service.unit_file.write_text('fixture')
        info = dict(LoadState='loaded', ActiveState='active', MainPID='42',
                    FragmentPath=str(service.unit_file), DropInPaths='',
                    KillMode='mixed', SendSIGKILL='yes', Delegate='no', NeedDaemonReload='no')
        with patch.object(service, 'show', return_value=info), \
             patch.object(service, 'executable', return_value=self.old / 'bin/framerize') as exe:
            service.validate(self.old, self.new)
            info['DropInPaths'] = '/unreviewed/override.conf'
            with self.assertRaisesRegex(ValueError, 'migration'):
                service.validate(self.old, self.new)
            info['DropInPaths'] = ''
            exe.return_value = Path('/unrelated/framerize')
            with self.assertRaisesRegex(ValueError, 'running executable'):
                service.validate(self.old, self.new)
