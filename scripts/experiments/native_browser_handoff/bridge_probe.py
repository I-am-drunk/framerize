"""Run the production Rust/Python/browser pipe against local synthetic peers."""
import json
import os
from pathlib import Path
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "shared_context_isolation"))
from memory import visible_processes


def run():
    for kind in ("net", "pid"):
        parent = os.environ.get("FIXTURE_PARENT_" + kind.upper())
        if not parent or os.readlink("/proc/self/ns/" + kind) == parent:
            raise RuntimeError("private network and PID namespaces are required")
    baseline = visible_processes()
    selected = ("transport::browser::host::tests::"
                "actual_pipe_keeps_supplied_result_and_cancellation_owned_through_browser_close")
    result = subprocess.run(["/native-tests", selected, "--exact", "--ignored", "--nocapture"],
                            capture_output=True, text=True, timeout=70)
    print(result.stdout, end="", flush=True)
    print(result.stderr, file=sys.stderr, end="", flush=True)
    remaining = None
    for _ in range(20):
        remaining = {pid: stamp for pid, stamp in visible_processes().items()
                     if baseline.get(pid) != stamp}
        if not remaining:
            break
        time.sleep(0.1)
    passed = result.returncode == 0 and "1 passed; 0 failed" in result.stdout and remaining == {}
    print(json.dumps(dict(status="passed" if passed else "failed", exit_code=result.returncode,
        cleanup_joined=remaining == {}, remaining_processes=sorted(remaining or {}))), flush=True)
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(run())
