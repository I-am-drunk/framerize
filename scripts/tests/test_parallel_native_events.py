import json
import os
from pathlib import Path
import tempfile
import unittest
from native_check_fixture import completion
from parallel_native_check.events import Events, clean_completion, clean_sends
from parallel_native_check.support import Refusal


class EventsTests(unittest.TestCase):
    def test_prior_day_writer_remains_valid_across_midnight(self):
        path, events = self.source(name='events-2026-10-23.ndjson')
        writer = self.enterContext(path.open('ab'))
        events.bind_writer(os.getpid())
        writer.write(json.dumps({'event': 'native.http.send', 'invocation': 1}).encode() + b'\n')
        writer.flush()
        events.poll()
        self.assertEqual(events.send_invocations, [1])

    def test_restart_to_new_daily_log_rejects_stale_plan(self):
        path, events = self.source(name='events-2026-10-23.ndjson')
        self.enterContext(path.with_name('events-2026-10-24.ndjson').open('ab'))
        with self.assertRaisesRegex(Refusal, 'event_writer_mismatch'):
            events.bind_writer(os.getpid())

    def test_read_only_open_file_cannot_attest_to_writer(self):
        path, events = self.source()
        self.enterContext(path.open('rb'))
        with self.assertRaisesRegex(Refusal, 'event_writer_mismatch'):
            events.bind_writer(os.getpid())

    def test_writer_replaced_after_binding_refuses_further_observation(self):
        path, events = self.source()
        writer = self.enterContext(path.open('ab'))
        events.bind_writer(os.getpid())
        other = self.enterContext(path.with_name('replacement.ndjson').open('ab'))
        os.dup2(other.fileno(), writer.fileno())
        with self.assertRaisesRegex(Refusal, 'event_writer_changed'):
            events.poll()

    def test_exact_send_and_cleanup_records_are_bounded_and_retained(self):
        path, events = self.source()
        with path.open('a') as stream:
            for count in (1, 2):
                stream.write(json.dumps({'event': 'native.http.send', 'invocation': count}) + '\n')
                stream.write(json.dumps({'event': 'native.stream.sends', 'sends': {
                    'send_invocations': count, 'cleanup_confirmed': True}}) + '\n')
        events.poll()
        self.assertEqual(events.send_invocations, [1, 2])
        self.assertEqual([row['send_invocations'] for row in events.send_snapshots], [1, 2])
        self.assertTrue(all(row['cleanup_confirmed'] for row in events.send_snapshots))

    def test_send_counter_rejects_gap_replay_boolean_and_third_send(self):
        for counts in ([2], [True], [1, 1], [1, 3], [1, 2, 3]):
            with self.subTest(counts=counts):
                path, events = self.source()
                with path.open('a') as stream:
                    for count in counts:
                        stream.write(json.dumps({'event': 'native.http.send', 'invocation': count}) + '\n')
                with self.assertRaisesRegex(Refusal, 'unexpected_native_send'):
                    events.poll()

    def test_send_snapshot_rejects_unbounded_or_private_fields(self):
        valid = {'send_invocations': 1, 'cleanup_confirmed': True}
        for value in (dict(valid, provider='private'), dict(valid, send_invocations=True),
                      dict(valid, send_invocations=-1), dict(valid, send_invocations=2**64),
                      dict(valid, cleanup_confirmed=1)):
            with self.subTest(value=value), self.assertRaises(Refusal):
                clean_sends(value)

    def source(self, history='', name='events.jsonl'):
        root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        logs = root / 'profiles/.logs'
        logs.mkdir(parents=True)
        path = logs / name
        path.write_text(history)
        events = Events(path, root)
        self.addCleanup(events.close)
        return path, events

    def test_only_new_bounded_completion_fields_are_reported(self):
        row = {'event': 'native.stream.complete', 'completion': completion()}
        path, events = self.source(json.dumps(row) + '\n')
        with path.open('a') as stream:
            stream.write(json.dumps({'event': 'ai.chain.repick', 'reason': 'private-reason'}) + '\n')
            stream.write(json.dumps(row) + '\n')
        events.poll()
        self.assertEqual(len(events.completed), 1)
        self.assertNotIn('private-reason', json.dumps(events.completed))

    def test_extra_private_fields_are_rejected(self):
        with self.assertRaisesRegex(Refusal, 'completion_shape'):
            clean_completion(dict(completion(), provider_header='private-header'))

    def test_invalid_scalars_never_enter_the_report(self):
        for key, value in (('body_failure', 'private-error'), ('observed_attempts', True),
                           ('last_status', 900), ('yielded_bytes', -1)):
            with self.subTest(key=key), self.assertRaises(Refusal):
                clean_completion(dict(completion(), **{key: value}))

    def test_partial_log_line_waits_for_its_end(self):
        path, events = self.source()
        row = json.dumps({'event': 'native.stream.complete', 'completion': completion()})
        with path.open('a') as stream:
            stream.write(row)
        events.poll()
        self.assertFalse(events.completed)
        with path.open('a') as stream:
            stream.write('\n')
        events.poll()
        self.assertEqual(len(events.completed), 1)

    def test_log_replacement_refuses_mixed_generations(self):
        path, events = self.source()
        path.rename(path.with_suffix('.old'))
        path.write_text('')
        with self.assertRaisesRegex(Refusal, 'event_file_changed'):
            events.poll()

    def test_already_saved_extra_completion_cannot_hide_in_unread_tail(self):
        path, events = self.source()
        row = json.dumps({'event': 'native.stream.complete', 'completion': completion()}) + '\n'
        with path.open('a') as stream:
            stream.write(row * 2 + (json.dumps({'event': 'ignored'}) + '\n') * 4000 + row)
        with self.assertRaisesRegex(Refusal, 'unexpected_native_completion'):
            events.poll()
