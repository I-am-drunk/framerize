import json
import os
from pathlib import Path
import time
from .support import require

KEYS = {'kind', 'body_failure', 'cleanup_confirmed', 'observed_attempts',
        'last_status', 'yielded_bytes', 'body_elapsed_ms'}
FAILURES = frozenset(('InvalidRequest UnsupportedStreaming UnsupportedResponse MissingContext '
    'MissingAuthorization InvalidScope ScopeClosed Deadline Cancelled Overloaded Authorization '
    'Transport Body BodyTooLarge Upstream TerminalFailure Cleanup Internal Ineligible '
    'PaidDisabled CreditFloor').split())


def clean_completion(value):
    require(isinstance(value, dict) and set(value) == KEYS, 'completion_shape')
    require(value['kind'] in ('raw', 'composed'), 'completion_kind')
    failure = value['body_failure']
    require(failure is None or isinstance(failure, str) and failure in FAILURES, 'completion_failure')
    require(type(value['cleanup_confirmed']) is bool, 'completion_cleanup')
    for key in ('observed_attempts', 'yielded_bytes', 'body_elapsed_ms'):
        require(type(value[key]) is int and 0 <= value[key] <= 2**64 - 1, 'completion_count')
    status = value['last_status']
    require(status is None or type(status) is int and 100 <= status <= 599, 'completion_status')
    return dict(value, observed_at_unix=time.time())


def clean_sends(value):
    require(isinstance(value, dict) and
            set(value) == {'send_invocations', 'cleanup_confirmed'}, 'send_snapshot_shape')
    count = value['send_invocations']
    require(type(count) is int and 0 <= count <= 2**64 - 1, 'send_snapshot_count')
    require(type(value['cleanup_confirmed']) is bool, 'send_snapshot_cleanup')
    return dict(value, observed_at_unix=time.time())


class Events:
    def __init__(self, path, root):
        self.path = Path(path)
        require(self.path.is_relative_to(root / 'profiles/.logs') and
                self.path.resolve(strict=True) == self.path, 'event_path')
        self.stream = self.path.open('rb')
        info = os.fstat(self.stream.fileno())
        self.identity = (info.st_dev, info.st_ino)
        self.stream.seek(0, 2)
        self.pending = b''
        self.bytes = 0
        self.completed = []
        self.send_invocations = []
        self.send_snapshots = []
        self.refusals = 0

    def poll(self):
        info = self.path.stat()
        require((info.st_dev, info.st_ino) == self.identity and
                info.st_size >= self.stream.tell(), 'event_file_changed')
        raw = self.stream.read(2 * 1024**2 - self.bytes + 1)
        self.bytes += len(raw)
        require(self.bytes <= 2 * 1024**2, 'event_byte_limit')
        self.pending += raw
        lines = self.pending.split(b'\n')
        self.pending = lines.pop()
        for line in lines:
            row = json.loads(line)
            require(isinstance(row, dict), 'event_shape')
            if row.get('event') == 'operation.admission.refused':
                self.refusals += 1
            elif row.get('event') == 'native.http.send':
                count = row.get('invocation')
                require(type(count) is int and count == len(self.send_invocations) + 1
                        and count <= 2, 'unexpected_native_send')
                self.send_invocations.append(count)
            elif row.get('event') == 'native.stream.sends':
                self.send_snapshots.append(clean_sends(row.get('sends')))
                require(len(self.send_snapshots) <= 2, 'unexpected_send_snapshot')
            elif row.get('event') == 'native.stream.complete':
                self.completed.append(clean_completion(row.get('completion')))
                require(len(self.completed) <= 2, 'unexpected_native_completion')

    def close(self):
        self.stream.close()
