import dataclasses
import json
import os
from pathlib import Path
import select
import subprocess
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import process_phase

CHILD = """
import sys, threading
print('idle', flush=True)
sys.stdin.readline()
ready = threading.Barrier(3)
stopped = threading.Event()
threads = [threading.Thread(target=lambda: (ready.wait(), stopped.wait())) for _ in range(2)]
for thread in threads: thread.start()
ready.wait()
print('active', flush=True)
sys.stdin.readline()
stopped.set()
for thread in threads: thread.join()
print('idle', flush=True)
sys.stdin.readline()
"""


@unittest.skipUnless(sys.platform == "linux", "requires Linux procfs")
class ProcessPhaseTests(unittest.TestCase):
    def _settle(self, child):
        if child.poll() is None:
            child.terminate()
        try:
            child.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            child.kill()
            child.communicate(timeout=5)

    def _line(self, child, expected):
        self.assertTrue(select.select([child.stdout], [], [], 5)[0], "child stalled")
        self.assertEqual(child.stdout.readline(), expected + "\n")

    def test_owned_process_active_and_idle_thread_counts(self):
        child = subprocess.Popen([sys.executable, "-I", "-B", "-u", "-c", CHILD],
                                 stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        self.addCleanup(self._settle, child)
        self._line(child, "idle")
        expected = process_phase.identity(child.pid)
        empty = process_phase.snapshot(expected, "empty")
        child.stdin.write("\n")
        child.stdin.flush()
        self._line(child, "active")
        active = process_phase.snapshot(expected, "active")
        self.assertEqual(active["threads"], empty["threads"] + 2)
        child.stdin.write("\n")
        child.stdin.flush()
        self._line(child, "idle")
        idle = process_phase.snapshot(expected, "idle")
        self.assertEqual(idle["threads"], empty["threads"])
        for sample in (empty, active, idle):
            self.assertEqual(sample["identity"], dataclasses.asdict(expected))
            self.assertGreater(sample["vm_size_kib"], 0)
            self.assertLessEqual(sample["started_monotonic_ns"], sample["finished_monotonic_ns"])
        child.stdin.write("\n")
        child.stdin.flush()
        self.assertEqual(child.wait(timeout=5), 0)
        with self.assertRaises(OSError):
            process_phase.snapshot(expected, "idle")

    def test_rejects_changed_identity_before_and_after_read(self):
        expected = process_phase.identity(os.getpid())
        changed = dataclasses.replace(expected, start_ticks=expected.start_ticks + 1)
        with self.assertRaisesRegex(process_phase.IncompleteSample, "process_identity_changed"):
            process_phase.snapshot(changed, "idle")
        with patch.object(process_phase, "identity", side_effect=[expected, changed]):
            with self.assertRaisesRegex(process_phase.IncompleteSample, "process_identity_changed"):
                process_phase.snapshot(expected, "idle")

    def test_rejects_incomplete_ambiguous_or_wrong_unit_counters(self):
        for text in ("Threads: 1\n", "VmSize: 1024 kB\n", "Threads: 0\nVmSize: 1 kB\n",
                     "Threads: 1\nVmSize: 1 MB\n", "Threads: 1\nVmSize: -1 kB\n",
                     "Threads: 1\nVmSize: 1 kB\nVmSize: 2 kB\n"):
            with self.subTest(text=text):
                with self.assertRaises(process_phase.IncompleteSample):
                    process_phase._status(text)

    def test_cli_reports_samples_and_refuses_stale_identity(self):
        expected = process_phase.identity(os.getpid())
        argv = [sys.executable, "-I", "-B", process_phase.__file__,
                "--pid", str(expected.pid), "--boot-id", expected.boot_id,
                "--phase", "idle", "--start-ticks", str(expected.start_ticks)]
        result = subprocess.run(argv, capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout)["identity"], dataclasses.asdict(expected))
        result = subprocess.run(argv[:-1] + [str(expected.start_ticks + 1)],
                                capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertEqual(json.loads(result.stdout),
                         {"ok": False, "error": "process_identity_changed"})


if __name__ == "__main__":
    unittest.main()
