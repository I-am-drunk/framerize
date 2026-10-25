"""One-time import of a complete legacy snapshot, never an empty replacement.

The caller must supply the final snapshot and inventory from the named source
runtime. This module validates and stores them; it cannot establish export
freshness. No normal startup path silently invokes this import.
"""
from copy import deepcopy
from dataclasses import asdict
import fcntl
import json
import os
from pathlib import Path
import re
import sqlite3
import tempfile
from .journal import RateJournal
from .rates import AccountRates, RatePolicy, SystemClock
from . import runtime_owner


LEGACY_FIELDS = {'cooling', 'requests', 'last_mint', 'reserve_day',
                 'reserve_used', 'primary_device', 'ambient_cooling',
                 'ambient_requests'}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def convert_legacy(snapshot, inventories, policy=None, clock=None):
    """Preserve recorded charges and identities; imported cleanup stays unknown."""
    policy, clock = policy or RatePolicy(), clock or SystemClock()
    require(type(snapshot) is dict and set(snapshot) == {
        'format', 'boot_id', 'captured_monotonic', 'state'}, 'invalid legacy snapshot')
    require(type(snapshot['format']) is int and snapshot['format'] == 1
            and snapshot['boot_id'] == clock.boot_id, 'legacy snapshot boot differs')
    captured = AccountRates._number(snapshot['captured_monotonic'])
    require(captured <= clock.now(), 'legacy snapshot is from the future')
    state = snapshot['state']
    require(type(state) is dict and set(state) == {'generation', 'accounts'},
            'invalid legacy host state')
    require(type(state['generation']) is str
            and re.fullmatch('[0-9a-f]{32}', state['generation']), 'invalid host generation')
    rows = state['accounts']
    require(type(rows) is dict and 0 < len(rows) <= 64
            and type(inventories) is dict and set(rows) == set(inventories),
            'legacy account inventory differs')
    records = {}
    for account, row in rows.items():
        RateJournal._account(account)
        require(type(row) is dict and set(row) == LEGACY_FIELDS,
                'invalid legacy account state')
        inventory = inventories[account]
        require(type(inventory) is dict and set(inventory) == {'devices', 'reserve'},
                'incomplete legacy device inventory')
        require(type(row['requests']) is dict and type(row['cooling']) is dict
                and None not in row['requests'] and None not in row['cooling'],
                'invalid legacy device rows')
        requests, cooling = list(row['requests'].items()), list(row['cooling'].items())
        if row['ambient_requests'] is not None:
            requests.append((None, row['ambient_requests']))
        if row['ambient_cooling'] is not None:
            cooling.append((None, row['ambient_cooling']))
        require(type(row['reserve_day']) is str and len(row['reserve_day']) <= 10,
                'invalid legacy reserve day')
        raw = dict(devices=inventory['devices'], reserve=inventory['reserve'],
            requests=[list(pair) for pair in requests], cooling=[list(pair) for pair in cooling],
            primary=row['primary_device'], reserve_day=row['reserve_day'],
            reserve_used=row['reserve_used'], last_mint=row['last_mint'],
            reserve_after=captured + (86400 if row['reserve_used'] else 0),
            pending_mint=False, clean=False, uncertain=True)
        record = dict(version=1, boot_id=clock.boot_id, saved_at=captured,
                      policy=asdict(policy), state=deepcopy(raw))
        validator = AccountRates(None, account, policy, clock)
        validator._restore(record)
        records[account] = record
    return records


def _check_inventory(root, inventories):
    entries = {p.name for p in root.iterdir() if p.name != '.rate-history.lock'}
    require(entries == set(inventories), 'account directory inventory differs')
    for account, inventory in inventories.items():
        directory = root / account
        require(directory.is_dir() and not directory.is_symlink(), 'invalid account directory')
        path = directory / 'framerize-devices.json'
        require(path.is_file() and not path.is_symlink(), 'missing saved device inventory')
        with path.open('rb') as stream:
            raw = stream.read(16385)
        require(len(raw) <= 16384, 'saved device inventory exceeds limit')
        stable = json.loads(raw)
        expected = [d for d in inventory['devices'] if d not in inventory['reserve']]
        require(type(stable) is list and stable == expected, 'saved device inventory differs')


def install_legacy(root, snapshot, inventories, source_runtime, policy=None, clock=None):
    """Install once inside the successor runtime, from caller-verified final input.

    The predecessor must have stopped under the same systemd service. Its
    metadata is retained for normal recovery; import never declares it clean.
    """
    root, clock = Path(root), clock or SystemClock()
    require(root.is_absolute() and root.resolve() == root and root.is_dir(),
            'migration requires an existing canonical account root')
    snapshot, inventories, source_runtime = deepcopy((snapshot, inventories, source_runtime))
    records = convert_legacy(snapshot, inventories, policy, clock)
    current = runtime_owner.capture()
    require(runtime_owner.previous_stopped(source_runtime, current, clock.boot_id),
            'migration requires the successor service runtime')
    flags = os.O_CREAT | os.O_RDWR | os.O_CLOEXEC | os.O_NOFOLLOW
    with os.fdopen(os.open(root / '.rate-history.lock', flags, 0o600), 'rb') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        _check_inventory(root, inventories)
        _publish(root, records, inventories, source_runtime, current)
    return len(records)


def _write_database(path, records, source_runtime, evidence=None):
    os.close(os.open(path, os.O_CREAT | os.O_EXCL | os.O_RDWR, 0o600))
    db = sqlite3.connect(path, timeout=0)
    try:
        db.execute('PRAGMA journal_mode=DELETE')
        db.execute('PRAGMA synchronous=FULL')
        with db:
            db.execute('BEGIN IMMEDIATE')
            RateJournal._create_schema(db)
            owner = json.dumps(source_runtime, allow_nan=False, separators=(',', ':'))
            db.executemany('INSERT INTO accounts(account, snapshot, runtime_owner) VALUES (?, ?, ?)',
                [(account, json.dumps(record, allow_nan=False, separators=(',', ':')), owner)
                 for account, record in records.items()])
            if evidence is not None:
                db.execute('CREATE TABLE migration_evidence (record TEXT NOT NULL)')
                db.execute('INSERT INTO migration_evidence VALUES (?)',
                    (json.dumps(evidence, allow_nan=False, separators=(',', ':')),))
    finally:
        db.close()
    with path.open('rb') as stream:
        os.fsync(stream.fileno())


def _publish(root, records, inventories, source_runtime, current, evidence=None,
             before_publish=None):
    with tempfile.TemporaryDirectory(prefix='.rate-migration-', dir=root.parent) as directory:
        staging = Path(directory) / 'history.sqlite3'
        if evidence is None:
            _write_database(staging, records, source_runtime)
        else:
            _write_database(staging, records, source_runtime, evidence)
        _check_inventory(root, inventories)
        require(runtime_owner.capture() == current, 'successor runtime changed')
        if before_publish is not None:
            before_publish()
        os.link(staging, root / 'rate-history.sqlite3')
        directory_fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
