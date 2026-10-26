"""Recover missing history conservatively, only inside an exclusive service."""
from dataclasses import asdict
import os
from . import runtime_owner
from .rates import AccountRates, SystemClock

JOURNAL_FILES = {'.rate-history.lock', 'rate-history.sqlite3',
                 'rate-history.sqlite3-wal', 'rate-history.sqlite3-shm',
                 'rate-history.sqlite3-journal'}


def parent_owns_sessions(root):
    path = root.parent.parent / '.sessions/.owner.lock'
    if path.is_symlink() or not path.is_file():
        return False
    info = path.stat()
    identity = (os.major(info.st_dev), os.minor(info.st_dev), info.st_ino)
    for line in (runtime_owner.PROC / 'locks').read_text().splitlines():
        fields = line.split()
        if len(fields) != 8 or fields[1:5] != ['FLOCK', 'ADVISORY', 'WRITE', str(os.getppid())]:
            continue
        major, minor, inode = fields[5].split(':')
        if (int(major, 16), int(minor, 16), int(inode)) == identity:
            return True
    return False


def recover_profiles(root, owner, policy, clock):
    from .expiry import _inventory
    clock = clock or SystemClock()
    AccountRates(None, 'validation', policy, clock)
    if (not root.is_absolute() or root.resolve() != root or owner is None
            or owner.get('boot_id') != clock.boot_id or not parent_owns_sessions(root)):
        raise RuntimeError('missing rate history recovery requires exclusive service and session ownership')
    inventory = _inventory(root, JOURNAL_FILES)
    now = clock.now()
    wait = max(600.0, policy.window, policy.cooldown, policy.mint_gap)
    if wait > policy.cooldown:
        raise ValueError('missing rate history recovery requires a cooldown covering the rate window')
    records = {}
    for account, devices in inventory.items():
        stable = devices['devices']
        # No historical events are invented. Every identity is held until
        # unknown short-term usage expires; daily reserves stay unavailable.
        # Retain the maximum debt even if the configured reserve cap is later raised.
        state = dict(devices=stable, reserve=[], requests=[],
            cooling=[[device, now + wait] for device in [None, *stable]],
            primary=None, reserve_day=clock.day(), reserve_used=64,
            reserve_after=now + 86400, last_mint=now, pending_mint=False,
            clean=True, uncertain=False)
        records[account] = dict(version=1, boot_id=clock.boot_id, saved_at=now,
                               policy=asdict(policy), state=state)
    if (runtime_owner.capture() != owner or not parent_owns_sessions(root)
            or _inventory(root, JOURNAL_FILES) != inventory):
        raise RuntimeError('missing rate history recovery ownership changed')
    return records
