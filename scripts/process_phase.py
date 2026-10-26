#!/usr/bin/env python3
"""Sample Linux thread count and virtual size for one identified process.

This is a point observation, not a peak, resident-memory or process-tree sum.
Run it outside the measured process and use the same observer for controls.
Capture identity(pid) once; pass it to snapshot at each established barrier.
The CLI requires that same PID, start tick and boot ID for every phase.
Pair these readings with the owned-tree RSS/PSS/private-memory sampler.
"""
import argparse
import datetime
import json
import time
from dataclasses import asdict, dataclass
from pathlib import Path


class IncompleteSample(RuntimeError):
    """The reading cannot be bound to the requested process."""


@dataclass(frozen=True)
class ProcessIdentity:
    pid: int
    start_ticks: int
    boot_id: str


def identity(pid):
    if pid <= 0:
        raise IncompleteSample("invalid_pid")
    stat = Path(f"/proc/{pid}/stat").read_text(encoding="utf-8", errors="replace")
    try:
        start_ticks = int(stat.rsplit(") ", 1)[1].split()[19])
    except (IndexError, ValueError) as error:
        raise IncompleteSample("invalid_process_identity") from error
    boot_id = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
    if not boot_id or start_ticks < 0:
        raise IncompleteSample("invalid_process_identity")
    return ProcessIdentity(pid, start_ticks, boot_id)


def _status(text):
    values = {}
    for line in text.splitlines():
        name, _, value = line.partition(":")
        if name not in {"Threads", "VmSize"}:
            continue
        parts = value.split()
        wanted = 1 if name == "Threads" else 2
        if (name in values or len(parts) != wanted or not parts[0].isdecimal()
                or (name == "VmSize" and parts[1] != "kB")):
            raise IncompleteSample("invalid_status_counters")
        values[name] = int(parts[0])
    if set(values) != {"Threads", "VmSize"} or values["Threads"] < 1:
        raise IncompleteSample("incomplete_status_counters")
    return {"threads": values["Threads"], "vm_size_kib": values["VmSize"]}


def snapshot(expected, phase):
    if phase not in {"empty", "active", "idle"}:
        raise IncompleteSample("invalid_phase")
    started_ns = time.monotonic_ns()
    started_at = datetime.datetime.now(datetime.timezone.utc).isoformat()
    if identity(expected.pid) != expected:
        raise IncompleteSample("process_identity_changed")
    counters = _status(Path(f"/proc/{expected.pid}/status").read_text(
        encoding="utf-8", errors="replace"))
    if identity(expected.pid) != expected:
        raise IncompleteSample("process_identity_changed")
    return {"ok": True, "identity": asdict(expected), "phase": phase,
            "started_at_utc": started_at, "started_monotonic_ns": started_ns,
            "finished_monotonic_ns": time.monotonic_ns(), **counters}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--start-ticks", type=int, required=True)
    parser.add_argument("--boot-id", required=True)
    parser.add_argument("--phase", choices=("empty", "active", "idle"), required=True)
    args = parser.parse_args()
    expected = ProcessIdentity(args.pid, args.start_ticks, args.boot_id)
    try:
        result = snapshot(expected, args.phase)
    except OSError:
        result = {"ok": False, "error": "proc_read_failed"}
    except IncompleteSample as error:
        result = {"ok": False, "error": str(error)}
    print(json.dumps(result, sort_keys=True))
    return 0 if result["ok"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
