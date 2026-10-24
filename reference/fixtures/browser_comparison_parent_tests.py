"""Inert parent ordering and history tests; no child, SDK or HTTP."""
import copy
import json
from pathlib import Path
import struct
import sys
import unittest
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'scripts'))
from browser_comparison_parent import run_once
from browser_comparison_history import reserve, extend

NONCE = 'a' * 32


def report():
    return dict(baseline=dict(accepted=True, status=200, content_type='Json',
        response_complete=True, complete_json=True, nonempty_text=True, body_bytes=20,
        transport_starts=1, transport_finishes=1, fetch_settled=True, refusal=None),
        native=dict(complete_json=True, status=200, attempts=1, authorization_calls=1,
        transport_starts=1, local_cleanup=True, failed=False, refusal=None),
        native_nonempty_text=True, browser_cleanup_confirmed=True, account_closed=True,
        failure=None)


class Socket:
    def __init__(self, *, done=True):
        values = [dict(command='ready', nonce=NONCE),
                  dict(command='result', nonce=NONCE, report=report())]
        if done:
            values.append(dict(command='done', nonce=NONCE, local_pair_success=True, failure=None))
        self.incoming = b''.join(self.frame(v) for v in values)
        self.outgoing = b''

    @staticmethod
    def frame(value):
        data = json.dumps(value).encode()
        return struct.pack('!I', len(data)) + data

    def recv(self, size):
        part, self.incoming = self.incoming[:size], self.incoming[size:]
        return part

    def sendall(self, data):
        self.outgoing += data

    def settimeout(self, _):
        pass


class Child:
    def __init__(self, events, *, done=True, forced=False):
        self.events, self.forced = events, forced
        self.socket = Socket(done=done)

    def launch(self):
        self.events.append('launch')
        return self.socket

    def close(self):
        self.events.append('close')
        return dict(exit_code=0, group_absent=True, forced=self.forced,
                    resources={'memory.swap.current': 0, 'memory.events': {}, 'pids.events': {}})


class Window:
    def __init__(self, events, child, fail=None):
        self.events, self.child, self.fail = events, child, fail
        self.final = None

    def admit(self):
        self.events.append('admit')
        if self.fail == 'admit':
            raise RuntimeError('private fixture exception')

    def session(self):
        self.events.append('session')
        return 'session=private-fixture'

    def record(self, value):
        assert value == report()
        assert b'recorded' not in self.child.socket.outgoing
        self.events.append('record')
        if self.fail == 'record':
            raise RuntimeError('private fixture exception')

    def finish(self, facts):
        assert self.events[-1] == 'close'
        self.events.append('finish')
        self.final = copy.deepcopy(facts)
        return self.fail != 'finish'


def attempt(*, fail=None, done=True, forced=False):
    events = []
    child = Child(events, done=done, forced=forced)
    owner = Window(events, child, fail)
    result = run_once(dict(nonce=NONCE), child, owner)
    assert 'private' not in json.dumps(result)
    return result, events, child, owner


class ParentTests(unittest.TestCase):
    def test_record_precedes_ack_and_finish_follows_process_cleanup(self):
        result, events, child, _ = attempt()
        self.assertTrue(result['accepted'])
        self.assertEqual(events, ['admit', 'launch', 'session', 'record', 'close', 'finish'])
        self.assertIn(b'recorded', child.socket.outgoing)

    def test_publication_failure_never_acknowledges_child(self):
        result, events, child, owner = attempt(fail='record')
        self.assertFalse(result['accepted'])
        self.assertNotIn(b'recorded', child.socket.outgoing)
        self.assertEqual(events[-2:], ['close', 'finish'])
        self.assertTrue(owner.final['start_possible'])

    def test_lost_final_reply_retains_uncertainty_after_record(self):
        result, _, _, owner = attempt(done=False)
        self.assertFalse(result['accepted'])
        self.assertTrue(result['recorded'])
        self.assertEqual(owner.final['failure'], 'attempt_incomplete')

    def test_prelaunch_refusal_never_reads_session(self):
        result, events, _, owner = attempt(fail='admit')
        self.assertFalse(result['start_possible'])
        self.assertEqual(events, ['admit', 'close'])
        self.assertIsNone(owner.final)

    def test_final_history_failure_is_not_success(self):
        result, _, _, _ = attempt(fail='finish')
        self.assertFalse(result['accepted'])
        self.assertEqual(result['failure'], 'history_publication_failed')

    def test_forced_descendant_cleanup_is_not_delivery_acceptance(self):
        result, _, _, owner = attempt(forced=True)
        self.assertFalse(result['accepted'])
        self.assertTrue(owner.final['cleanup']['forced'])


def snapshot():
    return dict(format=1, boot_id='fixture-boot', captured_monotonic=700,
                state=dict(generation='a'*32, accounts={'account': dict(
                    cooling={'device': 600}, requests={'device': [1, 2]},
                    last_mint=2, reserve_day='', reserve_used=0,
                    primary_device='device', ambient_cooling=None, ambient_requests=None)}))


class HistoryTests(unittest.TestCase):
    def call(self, value, now=1000):
        return reserve(value, 'account', {'account': ['device']}, boot_id='fixture-boot', now=now)

    def test_reservation_preserves_original_and_charges_both_slots(self):
        original = snapshot()
        result = self.call(original)
        self.assertEqual(original, snapshot())
        row = result['state']['accounts']['account']
        self.assertEqual(row['requests']['device'], [1, 2, 1000, 1000])
        self.assertEqual(row['cooling']['device'], 1600)

    def test_active_history_cannot_be_reset_by_a_fresh_context(self):
        original = snapshot()
        original['state']['accounts']['account']['requests']['device'].append(700)
        with self.assertRaises(ValueError):
            self.call(original)

    def test_cleanup_extends_hold_without_removing_charged_history(self):
        reserved = self.call(snapshot())
        result = extend(reserved, 'account', {'account': ['device']},
                        boot_id='fixture-boot', now=1020)
        row = result['state']['accounts']['account']
        self.assertEqual(row['cooling']['device'], 1620)
        self.assertEqual(row['requests']['device'], [1, 2, 1000, 1000])
        self.assertEqual(reserved['captured_monotonic'], 1000)


if __name__ == '__main__':
    unittest.main(verbosity=2)
