import tempfile
import unittest
import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_host.startup_failure import snapshot, explain


class StartupFailureTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.proc, self.cg = self.root/'proc', self.root/'cgroup'
        (self.proc/'self').mkdir(parents=True)
        (self.cg/'parent/child').mkdir(parents=True)
        (self.proc/'self/cgroup').write_text('0::/parent/child\n')

    def events(self, group, file, content, limit):
        node = self.cg/group
        (node/file).write_text(content)
        (node/('pids.max' if file == 'pids.events' else 'memory.max')).write_text(limit)

    def test_parent_task_limit_is_measured_without_exposing_the_path(self):
        self.events('parent', 'pids.events', 'max 2\n', '128')
        before = snapshot(self.proc, self.cg)
        self.events('parent', 'pids.events', 'max 3\n', '128')
        message = explain(RuntimeError('browser closed; token=private-fixture'),
                          before, snapshot(self.proc, self.cg))
        self.assertIn('task limit events increased by 1 during startup, limit 128 tasks', message)
        self.assertIn('browser closed', message)
        self.assertNotIn('private-fixture', message)
        self.assertNotIn(str(self.root), message)

    def test_old_or_reset_counts_do_not_explain_a_new_failure(self):
        self.events('parent', 'pids.events', 'max 2\n', '128')
        before = snapshot(self.proc, self.cg)
        for count in (2, 1):
            self.events('parent', 'pids.events', f'max {count}\n', '128')
            message = explain(RuntimeError('navigation failed'), before, snapshot(self.proc, self.cg))
            self.assertEqual(message, 'browser preparation failed: RuntimeError: navigation failed')

    def test_missing_baseline_does_not_invent_a_limit_hit(self):
        self.events('parent', 'pids.events', 'max 8\n', '128')
        message = explain(RuntimeError('closed'), {}, snapshot(self.proc, self.cg))
        self.assertEqual(message, 'browser preparation failed: RuntimeError: closed')

    def test_memory_kill_preserves_the_observed_limit(self):
        self.events('parent/child', 'memory.events', 'oom_kill 0\n', '2147483648')
        before = snapshot(self.proc, self.cg)
        self.events('parent/child', 'memory.events', 'oom_kill 1\n', '2147483648')
        message = explain(RuntimeError('closed'), before, snapshot(self.proc, self.cg))
        self.assertIn('out-of-memory kill events increased by 1', message)
        self.assertIn('limit 2147483648 bytes', message)

    def test_missing_or_unsafe_cgroups_are_optional(self):
        self.assertEqual(snapshot(self.root/'absent', self.cg), {})
        (self.proc/'self/cgroup').write_text('0::/parent/../escape\n')
        self.assertEqual(snapshot(self.proc, self.cg), {})
