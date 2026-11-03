"""A successful compile is insufficient when the package uses another binary."""
from pathlib import Path
import json
import sys
import tomllib
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
import package_identity


class PackageIdentityTests(unittest.TestCase):
    def test_capabilities_come_from_the_binary_for_both_write_modes(self):
        commit = 'a' * 40
        version = tomllib.loads((ROOT/'apps/framerize/Cargo.toml').read_text())['package']['version']
        expected = f'framerize {version} ({commit})'
        for writer in (1, 2):
            capabilities = dict(read_versions=[1, 2], write_version=writer)
            with self.subTest(writer=writer), patch.object(package_identity.subprocess,
                    'check_output', side_effect=[expected, json.dumps(capabilities)]):
                info = package_identity.identify(ROOT, 'framerize', Path('/fixture'), commit, False)
            self.assertEqual(info['checkpoint_history'], capabilities)
            self.assertEqual(info['build_variant'], 'default' if writer == 2 else 'legacy-checkpoint-writes')

    def test_missing_or_invalid_binary_capabilities_reject_packaging(self):
        commit = 'a' * 40
        version = tomllib.loads((ROOT/'apps/framerize/Cargo.toml').read_text())['package']['version']
        expected = f'framerize {version} ({commit})'
        for capabilities in ('', '{}', 'null', '[]',
                '{"read_versions":[1],"write_version":2}'):
            with self.subTest(capabilities=capabilities), patch.object(package_identity.subprocess,
                    'check_output', side_effect=[expected, capabilities]):
                with self.assertRaisesRegex(ValueError, 'binary checkpoint capabilities'):
                    package_identity.identify(ROOT, 'framerize', Path('/fixture'), commit, False)

    def test_exact_version_commit_and_dirty_state_are_required(self):
        commit = 'a' * 40
        for app in ['framerize', 'framerize-native']:
            version = tomllib.loads((ROOT/'apps'/app/'Cargo.toml').read_text())['package']['version']
            for dirty in [False, True]:
                expected = f'{app} {version} ({commit}' + (', dirty)' if dirty else ')')
                capabilities = dict(read_versions=[1, 2], write_version=2)
                output = lambda args, **kwargs: (json.dumps(capabilities)
                    if args[-1] == 'checkpoint-history' else expected)
                with self.subTest(app=app, dirty=dirty), patch.object(
                        package_identity.subprocess, 'check_output', side_effect=output):
                    info = package_identity.identify(ROOT, app, Path('/fixture'), commit, dirty)
                    self.assertEqual(info['version_output'], expected)
                    self.assertEqual(info['release_tag'], 'v' + version)
                    if app == 'framerize':
                        self.assertEqual(info['checkpoint_history'], capabilities)
                for wrong in [expected.replace(commit, 'b' * 40),
                              f'{app} {version} (unverified source; build with scripts/cargo_with_identity.py)',
                              expected.replace(version, '0.0.0'),
                              expected.replace(app, 'unrelated-app'),
                              expected.replace(', dirty)', ')') if dirty else expected.replace(')', ', dirty)')]:
                    with self.subTest(output=wrong), patch.object(
                            package_identity.subprocess, 'check_output', return_value=wrong):
                        with self.assertRaisesRegex(ValueError, 'binary identity differs'):
                            package_identity.identify(ROOT, app, Path('/fixture'), commit, dirty)
