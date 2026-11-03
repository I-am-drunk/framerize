"""Opt-in private Claude bodies, with bounded memory and disk retention."""
import fcntl
import json
import os
from pathlib import Path
import queue
import re
import stat
import threading
import time
import uuid

from .events import emit

RECORD_LIMIT = 8 * 1024**2
TOTAL_LIMIT = 64 * 1024**2
FILE_LIMIT = 128
_SECRET = re.compile(r'(?i)^(authorization|proxy.authorization|cookie|set.cookie|'
    r'cookie_header|session|castleToken|access.?token|refresh.?token|api.?key|password|bearer|token)$')
_BEARER = re.compile(r'(?i)\bbearer\s+[^\s"\',;]+')
_JWT = re.compile(r'\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b')
_ASSIGNMENT = re.compile(r'(?i)\b(castleToken|access_token|refresh_token|api_key|'
    r'cookie|cookie_header|session|password|authorization)\s*[=:]\s*[^\s,;]+')
_FILE = re.compile(r'^capture-[0-9]+-[a-f0-9]{32}\.json$')


def redact(value):
    if isinstance(value, dict):
        return {key: '<redacted>' if _SECRET.fullmatch(key) else redact(item)
                for key, item in value.items()}
    if isinstance(value, list):
        return [redact(item) for item in value]
    if isinstance(value, str):
        return _ASSIGNMENT.sub(r'\1=<redacted>',
            _JWT.sub('<redacted>', _BEARER.sub('Bearer <redacted>', value)))
    return value


class BodyCapture:
    def __init__(self, root=None):
        logs = os.environ.get('FRAMERIZE_LOG_DIR')
        self.root = Path(root) if root else Path(logs) / 'claude-bodies' if logs else None
        self.pending = queue.Queue(maxsize=2)
        self.thread = None
        self.last_failure = 0

    def enabled(self):
        if self.root is None:
            return False
        try:
            marker = (self.root / 'enabled').lstat()
            return (stat.S_ISREG(marker.st_mode) and marker.st_uid == os.getuid()
                    and not marker.st_mode & 0o077
                    and 0 <= time.time() - marker.st_mtime < 7200)
        except OSError:
            return False

    def report_failure(self, cause):
        now = time.monotonic()
        if now - self.last_failure >= 60:
            self.last_failure = now
            emit('diagnostic.body_capture', level='error', outcome='dropped', cause=cause)

    def record(self, path, job, phase, value):
        if path != '/ai/v3/messages/' or not self.enabled():
            return
        try:
            record = dict(at=time.time(), job=job, phase=phase, path=path,
                          body=redact(value))
            data = bytearray()
            for part in json.JSONEncoder(ensure_ascii=True).iterencode(record):
                part = part.encode()
                if len(data) + len(part) > RECORD_LIMIT:
                    data = bytearray(json.dumps(dict(at=record['at'], job=job,
                        phase=phase, path=path, body_omitted='record exceeds 8 MiB')).encode())
                    break
                data.extend(part)
            if self.thread is None:
                worker = threading.Thread(target=self._worker,
                    name='framerize-body-capture', daemon=True)
                worker.start()
                self.thread = worker
            self.pending.put_nowait(bytes(data))
        except queue.Full:
            self.report_failure('private body capture queue is full')
        except Exception:
            self.report_failure('private body capture could not encode the record')

    def _worker(self):
        while True:
            data = self.pending.get()
            try:
                self._write(data)
            except Exception:
                self.report_failure('private body capture file write failed')
            finally:
                del data
                self.pending.task_done()

    def _write(self, data):
        directory = os.open(self.root, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try:
            info = os.fstat(directory)
            if info.st_uid != os.getuid() or info.st_mode & 0o077:
                raise PermissionError('capture directory must be private')
            fcntl.flock(directory, fcntl.LOCK_EX | fcntl.LOCK_NB)
            files = []
            for name in os.listdir(directory):
                if not _FILE.fullmatch(name):
                    continue
                entry = os.stat(name, dir_fd=directory, follow_symlinks=False)
                if not stat.S_ISREG(entry.st_mode) or entry.st_uid != os.getuid():
                    raise PermissionError('unsafe capture entry')
                files.append((name, entry.st_size))
            files.sort()
            total = sum(size for _, size in files)
            while files and (total + len(data) > TOTAL_LIMIT or len(files) >= FILE_LIMIT):
                name, size = files.pop(0)
                os.unlink(name, dir_fd=directory)
                total -= size
            name = f'capture-{time.time_ns()}-{uuid.uuid4().hex}.json'
            descriptor = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL |
                                 os.O_NOFOLLOW, 0o600, dir_fd=directory)
            try:
                with os.fdopen(descriptor, 'wb') as output:
                    output.write(data)
            except BaseException:
                os.unlink(name, dir_fd=directory)
                raise
        finally:
            os.close(directory)

    def response(self, path, job, result):
        if path != '/ai/v3/messages/' or not self.enabled():
            return
        try:
            self._response(path, job, result)
        except Exception:
            self.report_failure('private body capture could not encode the response')

    def _response(self, path, job, result):
        captured = dict(result)
        body = captured.get('body')
        if isinstance(body, str):
            try:
                captured['body'] = json.loads(body)
            except ValueError:
                pass
        captured['complete_response'] = result.get('status', -1) >= 100
        self.record(path, job, 'response', captured)
