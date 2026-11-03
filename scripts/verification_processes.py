"""Snapshot Linux descendants, including children spawned by worker threads."""
from pathlib import Path


def descendants(parent, proc=Path('/proc')):
    found, pending = set(), [parent]
    while pending:
        pid = pending.pop()
        if pid in found:
            continue
        found.add(pid)
        for path in (proc / str(pid) / 'task').glob('*/children'):
            try:
                pending.extend(int(child) for child in path.read_text().split())
            except FileNotFoundError:
                continue
    return found
