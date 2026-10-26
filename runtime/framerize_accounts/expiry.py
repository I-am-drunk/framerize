"""Record expiry of legacy local rate limits under one idle successor.

This is a pre-admission API. It neither stops a service nor waits in a loop.
The caller keeps the same owner alive and starts no account work until install
succeeds. Closing the witness loses the proof; a new witness starts a new clock.
Local expiry does not establish upstream acceptance or clear a vendor refusal.
"""
from copy import deepcopy
from dataclasses import asdict
from datetime import date
import fcntl
import json
import os
from pathlib import Path
from . import migration, runtime_owner
from .journal import RateJournal
from .rates import AccountRates, RatePolicy, SystemClock


# The deployed 93dad965 host and its admission-only repair use these bounds.
LEGACY_WINDOW_SECONDS = 600.0


def _inventory(root, ignored=()):
    result = {}
    entries = [p for p in root.iterdir()
               if p.name != '.rate-history.lock' and p.name not in ignored]
    migration.require(0 < len(entries) <= 64, 'invalid account inventory size')
    for path in entries:
        RateJournal._account(path.name)
        migration.require(path.is_dir() and not path.is_symlink(), 'invalid account directory')
        saved = path / 'framerize-devices.json'
        migration.require(saved.is_file() and not saved.is_symlink(), 'missing saved devices')
        with saved.open('rb') as stream:
            raw = stream.read(16385)
        migration.require(len(raw) <= 16384, 'saved device inventory exceeds limit')
        devices = json.loads(raw)
        migration.require(type(devices) is list and 0 < len(devices) <= 64
            and all(type(d) is str and 1 <= len(d) <= 256 for d in devices)
            and len(set(devices)) == len(devices), 'invalid saved devices')
        result[path.name] = dict(devices=devices, reserve=[])
    return result


def _identity(path):
    stat = path.stat(follow_symlinks=False)
    return stat.st_dev, stat.st_ino


class ExpiryWitness:
    """Hold an idle legacy profile until its unknown short-term usage expires.

    The successor daemon must own session exclusion and defer all account work
    for this object's lifetime. Old binaries ignore our file lock: proof of
    their removal comes from the same-service successor check, not the lock.
    No witness can be saved, reopened or transferred to another process.
    """
    def __init__(self, root, source_runtime, policy=None, clock=None):
        self.root, self.clock = Path(root), clock or SystemClock()
        self.policy, self._lock = policy or RatePolicy(), None
        self._source = deepcopy(source_runtime)
        migration.require(self.root.is_absolute() and self.root.resolve() == self.root
            and self.root.is_dir(), 'expiry requires a canonical account root')
        AccountRates(None, 'validation', self.policy, self.clock)
        self.window = max(LEGACY_WINDOW_SECONDS, self.policy.window,
                          self.policy.cooldown, self.policy.mint_gap)
        try:
            flags = os.O_CREAT | os.O_RDWR | os.O_CLOEXEC | os.O_NOFOLLOW
            self._lock = os.fdopen(os.open(self.root / '.rate-history.lock', flags, 0o600), 'rb')
            fcntl.flock(self._lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self._root_identity = _identity(self.root)
            self._inventory = _inventory(self.root)
            self._owner = deepcopy(runtime_owner.capture())
            migration.require(runtime_owner.previous_stopped(
                self._source, self._owner, self.clock.boot_id),
                'expiry requires the successor service runtime')
            self._pid, self._boot = os.getpid(), self.clock.boot_id
            self._started = self._last = AccountRates._number(self.clock.now())
            self._day = self.clock.day()
            migration.require(type(self._day) is str
                and date.fromisoformat(self._day).isoformat() == self._day,
                'invalid expiry start day')
        except BaseException:
            self.close()
            raise

    def _observe(self):
        try:
            migration.require(self._lock is not None and not self._lock.closed,
                              'expiry witness is closed')
            stat = os.fstat(self._lock.fileno())
            migration.require(_identity(self.root) == self._root_identity
                and _identity(self.root / '.rate-history.lock') == (stat.st_dev, stat.st_ino),
                'expiry profile ownership changed')
            migration.require(os.getpid() == self._pid and self.clock.boot_id == self._boot
                and runtime_owner.capture() == self._owner, 'expiry runtime changed')
            migration._check_inventory(self.root, self._inventory)
            now = AccountRates._number(self.clock.now())
            migration.require(now >= self._last, 'expiry clock moved backwards')
            self._last = now
            return now
        except BaseException:
            self.close()
            raise

    def remaining(self):
        """Seconds still required. This does not sleep or admit account work."""
        return max(0.0, self._started + self.window - self._observe())

    def install(self):
        """Publish expired-history provenance and retain ownership until close."""
        now = self._observe()
        migration.require(now - self._started >= self.window, 'legacy usage has not expired')
        records = {}
        reserve_after = self._started + 86400.0
        for account, inventory in self._inventory.items():
            # Unknown historical charges are not synthesized as request events.
            # Expired temporary reserves are retired; stable identities survive.
            state = dict(devices=deepcopy(inventory['devices']), reserve=[],
                requests=[], cooling=[], primary=None, reserve_day=self._day,
                reserve_used=self.policy.reserves, reserve_after=reserve_after,
                last_mint=0.0, pending_mint=False, clean=True, uncertain=False)
            records[account] = dict(version=1, boot_id=self._boot, saved_at=now,
                                   policy=asdict(self.policy), state=state)
            AccountRates(None, account, self.policy, self.clock)._restore(records[account])
        evidence = dict(kind='legacy-expiry', version=1, boot_id=self._boot,
            source_runtime=self._source, successor_runtime=self._owner,
            started_at=self._started, completed_at=now, window_seconds=self.window,
            reserve_blocked_until=reserve_after, prior_usage='unknown-expired',
            prior_primary='unknown')
        try:
            migration._publish(self.root, records, self._inventory, self._owner,
                               self._owner, evidence, before_publish=self._observe)
        except BaseException:
            self.close()
            raise
        return len(records)

    def close(self):
        if self._lock is not None:
            self._lock.close()
            self._lock = None

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()
