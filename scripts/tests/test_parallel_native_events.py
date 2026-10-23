import json
from pathlib import Path
import tempfile
import unittest
from native_check_fixture import completion
from parallel_native_check.events import Events, clean_completion
from parallel_native_check.support import Refusal


class EventsTests(unittest.TestCase):
    def source(self, history=''):
        root = Path(self.enterContext(tempfile.TemporaryDirectory()))
        logs = root / 'profiles/.logs'
        logs.mkdir(parents=True)
        path = logs / 'events.jsonl'
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
