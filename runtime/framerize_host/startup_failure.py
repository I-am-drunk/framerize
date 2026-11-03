"""Explain startup failures using changes in actual cgroup event counters."""
from pathlib import Path
from .events import error_cause


def snapshot(proc=Path('/proc'), cgroups=Path('/sys/fs/cgroup')):
    result = {}
    try:
        group = next(s[3:] for s in (proc/'self/cgroup').read_text().splitlines()
                     if s.startswith('0::/'))
        node = cgroups / group.lstrip('/')
        if '..' in Path(group).parts:
            return result
        while node.is_relative_to(cgroups):
            counters = {}
            for file, key in [('pids.events', 'max'), ('memory.events', 'oom_kill')]:
                try:
                    values = dict(s.split() for s in (node/file).read_text().splitlines())
                    count = int(values[key])
                    limit = (node/('pids.max' if key == 'max' else 'memory.max')).read_text().strip()
                    counters[key] = (count, int(limit) if limit != 'max' else None)
                except (OSError, ValueError, KeyError):
                    pass
            result[str(node)] = counters
            if node == cgroups:
                break
            node = node.parent
    except (OSError, StopIteration):
        pass
    return result


def explain(error, before, after):
    facts = []
    for key, label, unit in [('max', 'task limit', 'tasks'),
                             ('oom_kill', 'out-of-memory kill', 'bytes')]:
        for group, values in after.items():
            previous = before.get(group, {}).get(key)
            current = values.get(key)
            if previous is None or current is None or current[0] <= previous[0]:
                continue
            limit = f', limit {current[1]} {unit}' if current[1] is not None else ''
            facts.append(f'cgroup {label} events increased by {current[0]-previous[0]} during startup{limit}')
            break
    cause = error_cause(error)
    return 'browser preparation failed: ' + '; '.join([*facts, cause])
