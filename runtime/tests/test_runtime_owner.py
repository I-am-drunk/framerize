"""Service ownership from synthetic kernel and manager observations."""
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_host.accounts import runtime_owner as owner


class RuntimeOwnerTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        self.proc, self.cgroup = root / 'proc', root / 'cgroup'
        self.group = '/user.slice/framerize.service'
        self.directory = self.cgroup / self.group.lstrip('/')
        self.directory.mkdir(parents=True)
        (self.directory / 'cgroup.procs').write_text('41\n42\n')
        for pid in (41, 42):
            path = self.proc / str(pid)
            path.mkdir(parents=True)
            (path / 'cgroup').write_text('0::' + self.group + '\n')
        boot = self.proc / 'sys/kernel/random/boot_id'
        boot.parent.mkdir(parents=True)
        boot.write_text('boot-one')
        self.info = dict(MainPID='41', ControlGroup=self.group, InvocationID='b' * 32,
                         Delegate='no', KillMode='mixed', SendSIGKILL='yes')
        for override in (patch.object(owner, 'PROC', self.proc),
                         patch.object(owner, 'CGROUP', self.cgroup),
                         patch.object(owner.os, 'getpid', return_value=42),
                         patch.object(owner.os, 'getppid', return_value=41),
                         patch.object(owner, '_service', side_effect=lambda _: self.info)):
            override.start()
            self.addCleanup(override.stop)

    def test_new_exclusive_service_lifetime_proves_previous_one_stopped(self):
        current = owner.capture()
        self.assertIsNotNone(current)
        previous = dict(current, invocation='a' * 32)
        self.assertTrue(owner.previous_stopped(previous, current, 'boot-one'))
        for old in (None, current, dict(previous, unit='other.service'),
                    dict(previous, cgroup='/other.service'), dict(previous, boot_id='other'),
                    dict(previous, invocation=True), dict(previous, extra='unknown')):
            with self.subTest(old=old):
                self.assertFalse(owner.previous_stopped(old, current, 'boot-one'))

    def test_leftover_process_or_delegated_subgroup_prevents_recovery(self):
        (self.directory / 'cgroup.procs').write_text('41\n42\n50\n')
        self.assertIsNone(owner.capture())
        (self.directory / 'cgroup.procs').write_text('41\n42\n')
        (self.directory / 'leftover').mkdir()
        self.assertIsNone(owner.capture())

    def test_manager_mismatch_or_weakened_cleanup_never_supplies_proof(self):
        original = dict(self.info)
        for key, value in (('MainPID', '99'), ('ControlGroup', '/other'),
                           ('Delegate', 'yes'), ('KillMode', 'process'),
                           ('SendSIGKILL', 'no'), ('InvocationID', '')):
            self.info = dict(original, **{key: value})
            with self.subTest(key=key):
                self.assertIsNone(owner.capture())

    def test_missing_kernel_state_or_different_parent_refuses_recovery(self):
        path = self.proc / '41/cgroup'
        path.write_text('0::/other.service\n')
        self.assertIsNone(owner.capture())
        path.unlink()
        self.assertIsNone(owner.capture())
        for group in ('0::/../bad.service\n', '0::/session.scope\n', '1:cpu:/x\n'):
            (self.proc / '42/cgroup').write_text(group)
            self.assertIsNone(owner.capture())
