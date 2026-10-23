import ast
import json
import stat
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from native_device import NativeDevices, Rejected
from fixture import ACCOUNT, LEASE, SECOND, SOURCE, tree

scope = {}
node = next(n for n in tree.body if isinstance(n, ast.ClassDef)
            and n.name == 'AccountLifecycle')
exec(compile(ast.Module(body=[node], type_ignores=[]), str(SOURCE), 'exec'), scope)
Lifecycle = scope['AccountLifecycle']


class Enrollment(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.host = SimpleNamespace(generation='a' * 32, accounts={}, lifecycles={})
        self.now = 1000.0
        self.api = NativeDevices(self.host, self.root, gap=2.5, window=600,
            capacity=180, cooldown=600, clock=lambda: self.now, lifecycle_type=Lifecycle)
        self.request = dict(account=ACCOUNT, enrollment=SECOND,
                            user='fixture-user', device='fixture-device')
        self.open = dict(account=ACCOUNT, lease=LEASE, generation='a' * 32)

    def test_initial_enrollment_requires_the_full_cooldown_before_open(self):
        receipt = self.api.enroll(self.request)
        self.assertEqual(receipt['cooldown_ms'], 600000)
        self.assertFalse(hasattr(self.host, '_cooldown_restoration'))
        with self.assertRaisesRegex(Rejected, 'cooling'):
            self.api.open(self.open)
        self.now = 1599.999
        with self.assertRaisesRegex(Rejected, 'cooling'):
            self.api.open(self.open)
        self.now = 1600.0
        grant = self.api.open(self.open)
        self.assertEqual(grant['device'], 'fixture-device')
        self.assertEqual(grant['generation'], self.host.generation)
        self.assertEqual(self.host.lifecycles[ACCOUNT].active_commands, 1)

    def test_enrollment_is_explicit_and_private(self):
        self.api.lifecycle_type = None
        with self.assertRaisesRegex(Rejected, 'unavailable'):
            self.api.enroll(self.request)
        self.assertFalse((self.root / ACCOUNT).exists())
        self.api.lifecycle_type = Lifecycle
        self.api.enroll(self.request)
        directory = self.root / ACCOUNT
        for name in ('framerize-devices.json', 'native-enrollment.json'):
            self.assertEqual(stat.S_IMODE((directory / name).stat().st_mode), 0o600)
        self.assertEqual(json.loads((directory / 'framerize-devices.json').read_text()),
                         ['fixture-device'])
        self.assertNotIn('fixture-user', (directory / 'native-enrollment.json').read_text())

    async def test_reenrollment_cannot_clear_unknown_cleanup_or_rate_history(self):
        self.api.enroll(self.request)
        self.now = 1600.0
        self.api.open(self.open)
        request = dict(self.open, budget_ms=16000, sequence=1, joined=False)
        await self.api.mint(request)
        self.api.minted(request)
        life = self.host.lifecycles[ACCOUNT]
        before = list(life.requests['fixture-device'])
        with self.assertRaisesRegex(Rejected, 'already has history'):
            self.api.enroll(self.request)
        self.assertEqual(list(life.requests['fixture-device']), before)
        self.assertTrue(life.native_grants[LEASE].uncertain)
        with self.assertRaises(Rejected):
            self.api.open(dict(self.open, lease=SECOND))

    def test_changed_device_file_refuses_even_when_primary_is_retained(self):
        self.api.enroll(self.request)
        path = self.root / ACCOUNT / 'framerize-devices.json'
        path.write_text(json.dumps(['fixture-device', 'unexpected-device']))
        self.now = 1600.0
        with self.assertRaisesRegex(Rejected, 'initial native device state changed'):
            self.api.open(self.open)

    def test_generation_change_and_persisted_profile_prevent_reenrollment(self):
        self.api.enroll(self.request)
        self.host.generation = 'b' * 32
        self.now = 2000.0
        with self.assertRaisesRegex(Rejected, 'rate restoration required'):
            self.api.open(self.open)
        self.host.lifecycles = {}
        with self.assertRaisesRegex(Rejected, 'profile already exists'):
            self.api.enroll(self.request)

    def test_existing_lifecycle_and_profile_are_never_replaced(self):
        life = Lifecycle()
        life.cooling['old-device'] = 8000.0
        self.host.lifecycles[ACCOUNT] = life
        with self.assertRaisesRegex(Rejected, 'already has history'):
            self.api.enroll(self.request)
        self.assertIs(self.host.lifecycles[ACCOUNT], life)
        self.assertEqual(life.cooling, {'old-device': 8000.0})
        self.host.lifecycles = {}
        directory = self.root / ACCOUNT
        directory.mkdir()
        sentinel = directory / 'retained.json'
        sentinel.write_text('retained')
        with self.assertRaisesRegex(Rejected, 'profile already exists'):
            self.api.enroll(self.request)
        self.assertEqual(sentinel.read_text(), 'retained')

    def test_failed_private_write_leaves_no_admitted_lifecycle(self):
        with patch('native_device.private_new', side_effect=OSError('injected')):
            with self.assertRaises(OSError):
                self.api.enroll(self.request)
        self.assertNotIn(ACCOUNT, self.host.lifecycles)
        with self.assertRaisesRegex(Rejected, 'profile already exists'):
            self.api.enroll(self.request)

    def test_bad_identity_or_symlink_refuses_without_overwriting_state(self):
        for change in (dict(user=' '), dict(device='bad\ndevice'),
                       dict(enrollment='not-a-uuid')):
            with self.subTest(change=change):
                with self.assertRaises((Rejected, ValueError)):
                    self.api.enroll(dict(self.request, **change))
                self.assertFalse((self.root / ACCOUNT).exists())
        target = self.root / 'untouched'
        target.mkdir()
        (self.root / ACCOUNT).symlink_to(target, target_is_directory=True)
        with self.assertRaisesRegex(Rejected, 'profile already exists'):
            self.api.enroll(self.request)
        self.assertEqual(list(target.iterdir()), [])

    def test_another_account_cannot_reuse_enrolled_login_or_device(self):
        self.api.enroll(self.request)
        for changes in (dict(device='another-device'), dict(user='another-user')):
            with self.subTest(changes=changes):
                request = dict(self.request, account=SECOND, **changes)
                with self.assertRaisesRegex(Rejected, 'identity already owned'):
                    self.api.enroll(request)
                self.assertFalse((self.root / SECOND).exists())
