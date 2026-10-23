import unittest
from unittest.mock import patch

import application_memory_roles as roles
import test_application_worker_memory as fixtures


class RoleTests(unittest.TestCase):
    def setUp(self):
        self.fixture = fixtures.ProcessMemoryTests()
        self.fixture.setUp()

    def test_attribution_reconciles_totals_and_keeps_fallback(self):
        self.fixture.files['/proc/13/smaps_rollup'] = PermissionError()
        classify = lambda pid: ('application' if pid == 12 else 'worker', (pid, None))
        sample = self.fixture.measure(classify)
        self.assertEqual(sample['roles']['worker'], dict(pss_with_rss_fallback=1228800,
            rss=1228800, processes=1, fallback_processes=1))
        for name in ('pss_with_rss_fallback', 'rss', 'processes', 'fallback_processes'):
            self.assertEqual(sample[name], sum(part[name] for part in sample['roles'].values()))

    def test_process_reuse_rejects_whole_sample(self):
        sequence = iter([('worker', (1, (2, 3))), ('worker', (9, (2, 3)))])
        self.assertIsNone(self.fixture.measure(lambda pid: next(sequence)))
        self.assertEqual(self.fixture.failures, {'IdentityChanged': 1})

    def configured_roles(self):
        classifier = roles.ProcessRoles.__new__(roles.ProcessRoles)
        classifier.known = {12: ('application', (5, (1, 2)))}
        classifier.executables = {(3, 4): 'worker'}
        return classifier

    def test_reused_known_pid_is_unknown_even_if_executable_matches(self):
        with patch.object(roles, 'process_identity', return_value=(6, (1, 2))):
            self.assertEqual(self.configured_roles()(12)[0], 'unknown')

    def test_worker_requires_device_and_inode_and_not_just_name(self):
        classifier = self.configured_roles()
        for identity, expected in [((5, (3, 4)), 'worker'),
                                   ((5, (8, 4)), 'unknown'), ((5, None), 'unknown')]:
            with self.subTest(identity=identity):
                with patch.object(roles, 'process_identity', return_value=identity):
                    self.assertEqual(classifier(13)[0], expected)

    def test_stat_name_parentheses_and_unreadable_executable(self):
        line = '12 (a name with )) ' + ' '.join(['S'] + ['0'] * 18 + ['456'])
        with patch.object(roles.Path, 'read_text', return_value=line):
            with patch.object(roles, 'executable_id', side_effect=PermissionError()):
                self.assertEqual(roles.process_identity(12), (456, None))

    def sample(self, values):
        return {'roles': {name: dict(pss_with_rss_fallback=size, rss=size,
            processes=1, fallback_processes=0) for name, size in values.items()}}

    def test_unknown_memory_is_included_but_known_helpers_are_separate(self):
        target = {}
        roles.record_attribution(target, self.sample(dict(application=20, host=30,
            owner=10, worker=100, unknown=40, measurement=70, wrapper=50)))
        self.assertEqual(target['product_and_unknown_peak'], 200)
        self.assertEqual(target['role_peaks']['measurement']['rss'], 70)

    def test_product_peak_uses_one_sample_instead_of_summing_role_peaks(self):
        target = {}
        first = self.sample(dict(application=100, worker=0))
        second = self.sample(dict(application=20, worker=70))
        roles.record_attribution(target, first)
        roles.record_attribution(target, second)
        self.assertEqual(target['product_and_unknown_peak'], 100)
        self.assertIs(target['at_product_and_unknown_peak'], first)
        self.assertEqual(target['role_peaks']['worker']['rss'], 70)
