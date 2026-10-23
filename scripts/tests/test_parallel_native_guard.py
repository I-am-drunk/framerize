import hashlib
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
import native_check_fixture
from parallel_native_check.guard import Binding, separate_accounts, separate_roots, support_bindings
from parallel_native_check.runner import claim_attempt
from parallel_native_check.support import Refusal, error_code, read_json


class GuardTests(unittest.TestCase):
    def root(self):
        return Path(self.enterContext(tempfile.TemporaryDirectory()))

    def test_same_login_with_different_account_id_is_refused(self):
        with self.assertRaisesRegex(Refusal, 'browser_account_overlap'):
            separate_accounts([('native', 'login')], [('browser', 'login')], 'native', 'login')
        separate_accounts([('native', 'login')], [('browser', 'other')], 'native', 'login')

    def test_unknown_browser_login_is_not_assumed_separate(self):
        with self.assertRaisesRegex(Refusal, 'browser_account_identity_missing'):
            separate_accounts([('native', 'login')], [('browser', None)], 'native', 'login')

    def test_nested_and_symlinked_roots_are_refused(self):
        root = self.root()
        (root / 'child').mkdir()
        (root / 'alias').symlink_to(root / 'child')
        for native in (root / 'child', root / 'alias'):
            with self.subTest(native=native.name), self.assertRaises(Refusal):
                separate_roots(native, root)

    def test_private_input_requires_matching_bytes_and_private_permissions(self):
        path = self.root() / 'private.json'
        raw = b'{"fixture": true}'
        path.write_bytes(raw)
        path.chmod(0o600)
        digest = hashlib.sha256(raw).hexdigest()
        self.assertEqual(read_json(path, private=True, digest=digest), {'fixture': True})
        path.write_text('{"fixture": false}')
        with self.assertRaisesRegex(Refusal, 'input_changed'):
            read_json(path, private=True, digest=digest)
        path.chmod(0o644)
        with self.assertRaisesRegex(Refusal, 'input_permissions'):
            read_json(path, private=True)

    def test_same_generation_cannot_be_claimed_twice(self):
        binding = SimpleNamespace(root=self.root(), plan={'invocation_id': 'a' * 32})
        claim_attempt(binding, {'fixture': True})
        with self.assertRaises(FileExistsError):
            claim_attempt(binding, {'fixture': True})

    def test_runtime_exception_text_is_never_a_public_error_code(self):
        self.assertEqual(error_code(RuntimeError('private-provider-secret')), 'RuntimeError')
        self.assertEqual(error_code(Refusal('local_failure')), 'local_failure')

    def test_support_manifest_cannot_omit_a_member_pin(self):
        root = self.root()
        raw = json.dumps({'member.py': 'a' * 64}).encode()
        digest = hashlib.sha256(raw).hexdigest()
        directory = root / 'runtime' / ('cooldown-support-' + digest[:16])
        directory.mkdir(parents=True)
        (directory / 'runtime-sources.json').write_bytes(raw)
        name = str((directory / 'runtime-sources.json').relative_to(root))
        member = str((directory / 'member.py').relative_to(root))
        plan = {'support_manifest': name, 'files': {name: digest, member: 'a' * 64}}
        support_bindings(plan, root)
        del plan['files'][member]
        with self.assertRaisesRegex(Refusal, 'support_member_missing'):
            support_bindings(plan, root)

    def test_resource_refusals_use_direct_fixture_counters(self):
        binding = Binding.__new__(Binding)
        binding.group = self.root()
        binding.plan = {'memory_max_bytes': 128 * 1024**2}
        values = {'memory.current': '10', 'memory.peak': '20', 'memory.swap.current': '0',
            'pids.current': '2', 'pids.peak': '3', 'memory.max': str(128 * 1024**2),
            'memory.swap.max': '0', 'pids.max': '32', 'memory.events': 'oom 0\noom_kill 0\n'}
        for name, value in values.items():
            (binding.group / name).write_text(value)
        self.assertEqual(binding.sample()['memory.swap.current'], 0)
        for name, value in (('memory.swap.current', '1'), ('pids.max', '64'),
                            ('memory.events', 'oom 1\n'), ('memory.max', 'max')):
            (binding.group / name).write_text(value)
            with self.subTest(name=name), self.assertRaises((Refusal, ValueError)):
                binding.sample()
            (binding.group / name).write_text(values[name])
