"""Synthetic inputs only; no service, account store or upstream access."""
import json
from pathlib import Path
import sys
import time
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))


def event(kind, **value):
    value = dict(value, type=kind)
    return ('event: ' + kind + '\ndata: ' + json.dumps(value) + '\n\n').encode()


def terminal(text='OK'):
    return event('response.completed', response={'status': 'completed',
        'output': [{'type': 'message', 'content': [{'type': 'output_text', 'text': text}]}]})


def completion(failure=None, cleanup=True):
    return {'kind': 'composed', 'body_failure': failure, 'cleanup_confirmed': cleanup,
        'observed_attempts': 1, 'last_status': 200, 'yielded_bytes': 20, 'body_elapsed_ms': 2}


def await_state(stream, predicate):
    deadline = time.monotonic() + 2
    while time.monotonic() < deadline:
        value = stream.snapshot()
        if predicate(value):
            return value
        time.sleep(0.001)
    raise AssertionError('synthetic stream did not reach expected state')


class FakeStream:
    def __init__(self, complete):
        self.cancelled = False
        self.state = {'status': 200, 'read_error': None, 'finished': complete,
                      'eof': complete, 'terminal_completions': int(complete), 'text_characters': 2}

    def snapshot(self):
        return dict(self.state)

    def close(self, cancel=False):
        self.cancelled |= cancel
        self.state['finished'] = True


def quiet_pool():
    return {'config': {'eligible_accounts': ['account'], 'active_capacity': 1},
            'residency': {'residents': []}}


class Scenario:
    def __init__(self, cleanup=True, emit=True):
        self.sent = []
        self.cleanup = cleanup
        self.emit = emit
        self.binding = SimpleNamespace(account='account', refresh=lambda: None,
                                       sample=lambda: {'memory.current': 1})
        self.events = SimpleNamespace(completed=[], refusals=0, pending=b'', poll=self.poll)
        self.report = {'post_calls': 0, 'samples': []}

    def send(self, binding, cancel, report):
        if self.sent:
            assert self.events.completed[0]['cleanup_confirmed'], 'second request preceded cleanup'
        report['post_calls'] += 1
        stream = FakeStream(not cancel)
        self.sent.append(stream)
        return stream

    def poll(self):
        if not self.emit or not self.sent or not self.sent[0].cancelled:
            return
        if not self.events.completed:
            self.events.completed.append(completion('Cancelled', self.cleanup))
        if len(self.sent) == 2 and len(self.events.completed) == 1:
            self.events.completed.append(completion())
