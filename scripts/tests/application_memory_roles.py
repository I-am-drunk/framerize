"""Attribute synthetic fixture processes without reading arguments or secrets."""
import json
import os
from pathlib import Path


def executable_id(path):
    value = path.stat()
    return value.st_dev, value.st_ino


def process_identity(pid):
    root = Path('/proc', str(pid))
    fields = (root / 'stat').read_text().rsplit(')', 1)[1].split()
    start = int(fields[19])
    try:
        executable = executable_id(root / 'exe')
    except PermissionError:
        executable = None
    return start, executable


class ProcessRoles:
    def __init__(self, application, host, source):
        self.known = {}
        for role, record in [('application', application), ('host', host)]:
            identity = process_identity(record['pid'])
            assert identity[0] == record['start'], 'fixture process changed'
            self.known[record['pid']] = role, identity
        for pid in (os.getpid(), os.getppid()):
            self.known[pid] = 'measurement', process_identity(pid)
        settings = json.loads((source / 'mounts.json').read_text())
        self.executables = {
            executable_id(source / 'owner-probe'): 'owner',
            executable_id(Path(settings['node'])): 'worker',
            executable_id(Path('/usr/bin/bwrap')): 'wrapper',
        }

    def __call__(self, pid):
        identity = process_identity(pid)
        known = self.known.get(pid)
        if known is not None:
            role = known[0] if known[1] == identity else 'unknown'
        else:
            role = self.executables.get(identity[1], 'unknown')
        return role, identity


def record_attribution(target, sample):
    # Unknown processes stay in the conservative product estimate.
    roles = sample['roles']
    included = [value for role, value in roles.items()
                if role not in ('measurement', 'wrapper')]
    size = sum(value['pss_with_rss_fallback'] for value in included)
    if size > target.get('product_and_unknown_peak', -1):
        target['product_and_unknown_peak'] = size
        target['at_product_and_unknown_peak'] = sample
    peaks = target.setdefault('role_peaks', {})
    for role, values in roles.items():
        if values['pss_with_rss_fallback'] > peaks.get(role, {}).get('pss_with_rss_fallback', -1):
            peaks[role] = dict(values)
