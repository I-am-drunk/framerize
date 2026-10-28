"""A successful compile is insufficient when the package uses another binary."""
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
import package_identity


class PackageIdentityTests(unittest.TestCase):
    def test_exact_version_commit_and_dirty_state_are_required(self):
        commit = 'a' * 40
        for app in ['framerize', 'framerize-native']:
            for dirty in [False, True]:
                expected = f'{app} 0.6.0-alpha.1 ({commit}' + (', dirty)' if dirty else ')')
                with self.subTest(app=app, dirty=dirty), patch.object(
                        package_identity.subprocess, 'check_output', return_value=expected):
                    info = package_identity.identify(ROOT, app, Path('/fixture'), commit, dirty)
                    self.assertEqual(info['version_output'], expected)
                    self.assertEqual(info['release_tag'], 'v0.6.0-alpha.1')
                for wrong in [expected.replace(commit, 'b' * 40),
                              f'{app} 0.6.0-alpha.1 (unverified source; build with scripts/cargo_with_identity.py)',
                              expected.replace('0.6.0-alpha.1', '0.1.0'),
                              expected.replace(app, 'unrelated-app'),
                              expected.replace(', dirty)', ')') if dirty else expected.replace(')', ', dirty)')]:
                    with self.subTest(output=wrong), patch.object(
                            package_identity.subprocess, 'check_output', return_value=wrong):
                        with self.assertRaisesRegex(ValueError, 'binary identity differs'):
                            package_identity.identify(ROOT, app, Path('/fixture'), commit, dirty)
