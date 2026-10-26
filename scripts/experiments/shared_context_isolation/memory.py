"""Sample complete owned process trees; reject races instead of undercounting."""
import os
from pathlib import Path


def identity(pid):
    return int(Path(f"/proc/{pid}/stat").read_text().rsplit(") ", 1)[1].split()[19])


def visible_processes():
    return {int(p.name): identity(int(p.name)) for p in Path("/proc").iterdir()
            if p.name.isdecimal()}


def owned_processes():
    found, pending = {}, [os.getpid()]
    while pending:
        pid = pending.pop()
        if pid in found:
            continue
        found[pid] = identity(pid)
        for task in Path(f"/proc/{pid}/task").iterdir():
            pending.extend(map(int, (task / "children").read_text().split()))
    return found


def memory_sample():
    before = owned_processes()
    membership = Path("/proc/self/cgroup").read_text()
    if not membership.startswith("0::/") or len(membership.splitlines()) != 1:
        raise ValueError("cgroup v2 required")
    group = Path("/sys/fs/cgroup" + membership.strip()[3:])
    if (group / "memory.max").read_text().strip() != "2147483648":
        raise ValueError("2 GiB memory limit required")
    if (group / "memory.swap.max").read_text().strip() != "0":
        raise ValueError("swap must be disabled for the fixture")
    totals = dict(rss_kib=0, pss_kib=0, private_kib=0)
    required = {"Rss", "Pss", "Private_Clean", "Private_Dirty"}
    for pid, stamp in before.items():
        if Path(f"/proc/{pid}/cgroup").read_text() != membership:
            raise ValueError("owned process escaped fixture cgroup")
        fields = {}
        for line in Path(f"/proc/{pid}/smaps_rollup").read_text().splitlines():
            name, _, rest = line.partition(":")
            if name not in required:
                continue
            value, unit = rest.split()
            if name in fields or unit != "kB" or int(value) < 0:
                raise ValueError("invalid memory accounting")
            fields[name] = int(value)
        if fields.keys() != required or identity(pid) != stamp:
            raise ValueError("incomplete memory accounting")
        totals["rss_kib"] += fields["Rss"]
        totals["pss_kib"] += fields["Pss"]
        totals["private_kib"] += fields["Private_Clean"] + fields["Private_Dirty"]
    if owned_processes() != before:
        raise ValueError("process tree changed during sample")
    totals["cgroup_memory_bytes"] = int((group / "memory.current").read_text())
    totals["cgroup_peak_bytes"] = int((group / "memory.peak").read_text())
    totals["cgroup_swap_bytes"] = int((group / "memory.swap.current").read_text())
    totals["cgroup_tasks"] = int((group / "pids.current").read_text())
    totals["cgroup_task_limit"] = int((group / "pids.max").read_text())
    totals["cgroup_task_limit_hits"] = int((group / "pids.events").read_text().split()[1])
    totals["cgroup_memory_events"] = {name: int(value) for name, value in
                                     (row.split() for row in (group / "memory.events").read_text().splitlines())}
    return dict(totals, process_count=len(before))


async def snapshot():
    import asyncio
    for rejected in range(10):
        try:
            return dict(memory_sample(), rejected_samples=rejected)
        except (OSError, ValueError, ProcessLookupError):
            await asyncio.sleep(0.02)
    raise RuntimeError("no complete memory sample")
