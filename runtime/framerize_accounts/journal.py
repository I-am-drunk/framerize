"""One durable, exclusively owned rate journal, separate from release files."""
import fcntl
import json
import os
from pathlib import Path
import re
import sqlite3
from . import runtime_owner


class RateJournal:
    def __init__(self, root, *, recovery_policy=None, clock=None):
        self.root = Path(root)
        self.root.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.connection = None
        self.lock = None
        self.recovery_policy, self.clock = recovery_policy, clock
        self.recovered_accounts = 0
        try:
            fd = os.open(self.root / '.rate-history.lock',
                         os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
            self.lock = os.fdopen(fd, 'rb')
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            path = self.root / 'rate-history.sqlite3'
            self.runtime_owner = runtime_owner.capture()
            fresh = not path.exists()
            records = self._missing_records() if fresh else {}
            if fresh:
                os.close(os.open(path, os.O_CREAT | os.O_EXCL | os.O_RDWR, 0o600))
            if path.is_symlink() or not path.is_file():
                raise RuntimeError('rate-history must be a regular file')
            self.connection = sqlite3.connect(path, timeout=0)
            self._initialize(fresh, records)
        except BaseException:
            self.close()
            raise

    def _missing_records(self):
        from .journal_recovery import JOURNAL_FILES, recover_profiles
        if not any(p.name not in JOURNAL_FILES for p in self.root.iterdir()):
            return {}
        if self.recovery_policy is None:
            raise RuntimeError('existing account state requires rate-history migration')
        return recover_profiles(self.root, self.runtime_owner, self.recovery_policy, self.clock)

    def _initialize(self, fresh, records):
        db = self.connection
        version = db.execute('PRAGMA user_version').fetchone()[0]
        from .journal_schema import schema_version
        version = schema_version(db, version)
        if version == 0:
            if not fresh:
                records = self._missing_records()
            fresh = True
        db.execute('PRAGMA journal_mode=WAL')
        db.execute('PRAGMA synchronous=FULL')
        if fresh:
            with db:
                db.execute('BEGIN IMMEDIATE')
                self._create_schema(db)
                owner = json.dumps(self.runtime_owner, separators=(',', ':'))
                db.executemany('INSERT INTO accounts VALUES (?, ?, ?)', [
                    (account, json.dumps(record, allow_nan=False), owner)
                    for account, record in records.items()])
            self.recovered_accounts = len(records)
            directory = os.open(self.root, os.O_RDONLY | os.O_DIRECTORY)
            try:
                os.fsync(directory)
            finally:
                os.close(directory)
        elif version == 1:
            with db:
                db.execute('BEGIN IMMEDIATE')
                db.execute('ALTER TABLE accounts ADD COLUMN runtime_owner TEXT')
                db.execute('PRAGMA user_version=2')
        elif db.execute('PRAGMA user_version').fetchone()[0] == 0:
            with db:
                db.execute('PRAGMA user_version=2')

    @staticmethod
    def _create_schema(db):
        db.execute('CREATE TABLE accounts '
                   '(account TEXT PRIMARY KEY, snapshot TEXT NOT NULL, runtime_owner TEXT)')
        db.execute('PRAGMA user_version=2')

    @staticmethod
    def _account(account):
        if type(account) is not str or not re.fullmatch(r'[A-Za-z0-9_-]{1,128}', account):
            raise ValueError('invalid account journal key')

    def load(self, account):
        self._account(account)
        row = self.connection.execute(
            'SELECT snapshot FROM accounts WHERE account=?', (account,)).fetchone()
        if row is None:
            if (self.root / account).exists():
                raise RuntimeError('account has no durable rate history')
            return None
        return json.loads(row[0])

    def save(self, account, snapshot):
        self._account(account)
        raw = json.dumps(snapshot, allow_nan=False, separators=(',', ':'))
        owner = json.dumps(self.runtime_owner, separators=(',', ':'))
        with self.connection:
            self.connection.execute(
                'INSERT INTO accounts(account, snapshot, runtime_owner) VALUES (?, ?, ?) '
                'ON CONFLICT(account) DO UPDATE SET '
                'snapshot=excluded.snapshot, runtime_owner=excluded.runtime_owner',
                (account, raw, owner))

    def previous_runtime_stopped(self, account, boot):
        self._account(account)
        row = self.connection.execute(
            'SELECT runtime_owner FROM accounts WHERE account=?', (account,)).fetchone()
        previous = json.loads(row[0]) if row and row[0] is not None else None
        return runtime_owner.previous_stopped(previous, self.runtime_owner, boot)

    def close(self):
        try:
            if self.connection is not None:
                self.connection.close()
                self.connection = None
        finally:
            if self.lock is not None:
                self.lock.close()
                self.lock = None

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()
