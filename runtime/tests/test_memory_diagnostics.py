"""Memory refusal diagnostics use matched measurements without live reads."""
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_host.resources import memory_snapshot, memory_pressure_message


class MemoryDiagnosticTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        root = Path(self.directory.name)
        self.proc, self.cg = root / "proc", root / "cgroup"
        (self.proc / "self").mkdir(parents=True)
        (self.cg / "parent/child").mkdir(parents=True)

    def write(self, path, value):
        path.write_text(str(value))

    def snapshot(self):
        return memory_snapshot(self.proc, self.cg)

    def test_host_refusal_reports_used_total_and_reservations(self):
        self.write(self.proc / "meminfo", "MemTotal: 16777216 kB\nMemAvailable: 1048576 kB\n")
        snapshot = self.snapshot()
        message = memory_pressure_message(snapshot, 64 * 1024**2, 1024**3)
        self.assertIn("host RAM used/unavailable: 15.00 of 16.00 GiB", message)
        self.assertIn("headroom -120.41 MiB, reserved 64.00 MiB, new allocation needs 1024.00 MiB", message)

    def test_fixed_host_reserve_boundary_does_not_scale_with_ram(self):
        for total_kib in (8_388_608, 33_554_432):
            for available_kib, expected in ((1_171_874, -1024), (1_171_875, 0),
                                            (1_171_876, 1024), (1_300_000, 131_200_000)):
                with self.subTest(total=total_kib, available=available_kib):
                    self.write(self.proc / "meminfo",
                               f"MemTotal: {total_kib} kB\nMemAvailable: {available_kib} kB\n")
                    snapshot = self.snapshot()
                    self.assertEqual(snapshot["host_reserve_bytes"], 1_200_000_000)
                    self.assertEqual(snapshot["allocation_headroom_bytes"], expected)

    def test_tightest_ancestor_pairs_its_own_usage_and_limit(self):
        self.write(self.proc / "self/cgroup", "0::/parent/child\n")
        self.write(self.proc / "meminfo", "MemTotal: 33554432 kB\nMemAvailable: 16777216 kB\n")
        for name, used, total in (("parent/child", 1, 4), ("parent", 15, 16)):
            self.write(self.cg / name / "memory.current", used * 1024**3)
            self.write(self.cg / name / "memory.max", total * 1024**3)
        snapshot = self.snapshot()
        self.assertEqual(snapshot["cgroup_current_bytes"], 1024**3)
        self.assertEqual(snapshot["cgroup_limit_bytes"], 4 * 1024**3)
        message = memory_pressure_message(snapshot, 0, 1024**3)
        self.assertIn("limiting cgroup RAM used: 15.00 of 16.00 GiB", message)
        self.assertNotIn("1.00 of 4.00", message)
        self.write(self.cg / "parent/memory.max", "max")
        message = memory_pressure_message(self.snapshot(), 0, 1024**3)
        self.assertIn("limiting cgroup RAM used: 1.00 of 4.00 GiB", message)

    def test_missing_usage_does_not_invent_a_measurement(self):
        self.assertEqual(self.snapshot(), {})
        message = memory_pressure_message({"allocation_headroom_bytes": 0}, 0, 1024**3)
        self.assertIn("RAM usage unavailable", message)
        self.assertNotIn("of", message)
