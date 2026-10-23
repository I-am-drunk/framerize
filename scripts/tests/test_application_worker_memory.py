"""Reject incomplete process samples and account for denied detailed maps."""
from pathlib import Path
import unittest
from unittest.mock import patch

from run_application_worker import process_memory


class ProcessMemoryTests(unittest.TestCase):
    def setUp(self):
        self.members = ['12 13', '12 13']
        self.files = {
            '/proc/12/smaps_rollup': 'Rss: 1000 kB\nPss: 600 kB\n',
            '/proc/13/smaps_rollup': 'Rss: 1200 kB\nPss: 400 kB\n',
            '/proc/12/status': 'Name: fixture\nVmRSS: 1000 kB\n',
            '/proc/13/status': 'Name: fixture\nVmRSS: 1200 kB\n',
        }
        self.failures = {}

    def measure(self, classify=None):
        def read(path):
            if str(path) == '/group/cgroup.procs':
                return self.members.pop(0)
            value = self.files[str(path)]
            if isinstance(value, Exception):
                raise value
            return value
        with patch.object(Path, 'read_text', read):
            return process_memory(Path('/group'), self.failures, classify)

    def test_readable_maps_keep_pss_separate_from_rss(self):
        self.assertEqual(self.measure(), dict(pss_with_rss_fallback=1024000,
            rss=2252800, processes=2, fallback_processes=0))
        self.assertEqual(self.failures, {})

    def test_denied_map_adds_full_rss_and_counts_fallback(self):
        self.files['/proc/13/smaps_rollup'] = PermissionError()
        self.assertEqual(self.measure(), dict(pss_with_rss_fallback=1843200,
            rss=2252800, processes=2, fallback_processes=1))

    def test_all_denied_maps_use_rss_for_every_process(self):
        for pid in (12, 13):
            self.files[f'/proc/{pid}/smaps_rollup'] = PermissionError()
        self.assertEqual(self.measure(), dict(pss_with_rss_fallback=2252800,
            rss=2252800, processes=2, fallback_processes=2))

    def test_unreadable_rss_rejects_whole_sample(self):
        self.files['/proc/13/smaps_rollup'] = PermissionError()
        self.files['/proc/13/status'] = PermissionError()
        self.assertIsNone(self.measure())
        self.assertEqual(self.failures, {'PermissionError': 1})

    def test_exited_process_rejects_whole_sample(self):
        self.files['/proc/13/smaps_rollup'] = FileNotFoundError()
        self.assertIsNone(self.measure())
        self.assertEqual(self.failures, {'FileNotFoundError': 1})

    def test_membership_change_rejects_whole_sample(self):
        self.members[1] = '12 13 14'
        self.assertIsNone(self.measure())
        self.assertEqual(self.failures, {'MembershipChanged': 1})

    def test_missing_pss_is_not_silently_counted_as_measured(self):
        self.files['/proc/13/smaps_rollup'] = 'Rss: 1200 kB\n'
        self.assertIsNone(self.measure())
        self.assertEqual(self.failures, {'KeyError': 1})

    def test_malformed_memory_rejects_whole_sample(self):
        self.files['/proc/13/smaps_rollup'] = 'Rss: unknown kB\nPss: 400 kB\n'
        self.assertIsNone(self.measure())
        self.assertEqual(self.failures, {'ValueError': 1})
