import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest


REPO = Path(__file__).resolve().parents[2]


def executable(path, content):
    path.write_text(content)
    path.chmod(0o755)


class WatchdogTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="framerize-watchdog-test-")
        self.root = Path(self.temporary.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        executable(self.bin / "df", "#!/bin/sh\nprintf '80%%\\n'\n")
        executable(self.bin / "free", "#!/bin/sh\nprintf 'Mem: 1000 700 0 0 100 200\\n'\n")
        executable(self.bin / "timeout", "#!/bin/sh\nexit 0\n")
        executable(self.bin / "pgrep", "#!/bin/sh\nexit 0\n")
        executable(self.bin / "sleep", "#!/bin/sh\nexit 0\n")
        executable(self.bin / "curl", '#!/bin/sh\n[ -f "$READY_FILE" ]\n')
        executable(self.bin / "systemctl", '#!/bin/sh\nprintf "%s\\n" "$*" >> "$SYSTEMCTL_LOG"\nif [ "$3" = restart ]; then touch "$READY_FILE"; fi\n')
        self.log = self.root / "watchdog.log"
        self.calls = self.root / "systemctl.log"
        self.ready = self.root / "ready"
        self.env = os.environ | {
            "PATH": str(self.bin) + os.pathsep + "/usr/bin:/bin",
            "FRAMERIZE_WATCHDOG_LOG": str(self.log),
            "FRAMERIZE_WATCHDOG_HEALTH_URL": "http://127.0.0.1:8792/api/ready",
            "READY_FILE": str(self.ready),
            "SYSTEMCTL_LOG": str(self.calls),
        }

    def tearDown(self):
        self.temporary.cleanup()

    def run_watchdog(self):
        return subprocess.run(["bash", str(REPO / "scripts/linux/iris-watchdog.sh")],
            env=self.env, text=True, capture_output=True, timeout=20)

    def test_low_memory_does_not_restart_ready_service(self):
        self.ready.touch()
        result = self.run_watchdog()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(self.calls.exists())
        self.assertIn("Framerize is ready — no restart", self.log.read_text())

    def test_low_memory_restart_waits_for_readiness_recovery(self):
        result = self.run_watchdog()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("restart framerize.service", self.calls.read_text())
        log = self.log.read_text()
        self.assertIn("requesting graceful restart", log)
        self.assertIn("readiness recovered after restart", log)


class DaemonCtlTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="framerize-daemonctl-test-")
        self.root = Path(self.temporary.name)
        self.binary = self.root / "framerize/target/debug/framerize"
        self.binary.parent.mkdir(parents=True)
        self.stopped = self.root / "stopped"
        executable(self.binary, "#!/bin/bash\ntrap 'touch \"$STOP_MARKER\"; /bin/sleep 0.5; exit 0' TERM\nwhile :; do /bin/sleep 1 & wait $! || true; done\n")
        self.env = os.environ | {
            "FRAMERIZE_ROOT": str(self.root),
            "FRAMERIZE_STOP_TIMEOUT_S": "5",
            "STOP_MARKER": str(self.stopped),
        }
        self.daemon = REPO / "scripts/daemonctl.sh"
        self.unrelated = None

    def tearDown(self):
        if self.unrelated and self.unrelated.poll() is None:
            self.unrelated.terminate()
            self.unrelated.wait(timeout=5)
        pidfile = self.root / "daemon.pid"
        if pidfile.exists():
            try:
                pid = int(pidfile.read_text())
                os.killpg(pid, 15)
            except (ValueError, ProcessLookupError, PermissionError):
                pass
        self.temporary.cleanup()

    def test_stop_waits_for_its_process_group_and_preserves_unrelated_browser(self):
        code = ("import ctypes,time; "
            "ctypes.CDLL(None).prctl(15, b'chrome-headless', 0, 0, 0); "
            "time.sleep(60)")
        self.unrelated = subprocess.Popen([sys.executable, "-c", code], start_new_session=True)
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            comm = Path(f"/proc/{self.unrelated.pid}/comm")
            if comm.exists() and comm.read_text().strip() == "chrome-headless":
                break
            time.sleep(0.02)
        self.assertEqual(Path(f"/proc/{self.unrelated.pid}/comm").read_text().strip(),
            "chrome-headless")

        started = subprocess.run(["bash", str(self.daemon), "start"], env=self.env,
            text=True, capture_output=True, timeout=10)
        self.assertEqual(started.returncode, 0, started.stderr)
        pidfile = self.root / "daemon.pid"
        pid = int(pidfile.read_text())
        process_info = subprocess.check_output(["ps", "-o", "pgid=,args=", "-p", str(pid)], text=True)
        start = time.monotonic()
        stopped = subprocess.run(["bash", str(self.daemon), "stop"], env=self.env,
            text=True, capture_output=True, timeout=10)
        elapsed = time.monotonic() - start

        self.assertEqual(stopped.returncode, 0, stopped.stdout + stopped.stderr + process_info)
        self.assertGreaterEqual(elapsed, 0.35)
        self.assertTrue(self.stopped.exists())
        self.assertFalse(pidfile.exists())
        self.assertIsNone(self.unrelated.poll())
        with self.assertRaises(ProcessLookupError):
            os.killpg(pid, 0)


if __name__ == "__main__":
    unittest.main()
