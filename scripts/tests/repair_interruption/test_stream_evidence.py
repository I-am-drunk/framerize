import json, sys, types, unittest
from unittest.mock import Mock, patch
from frames import Frames
from test_acceptance import ROOT, OTHER, state

def forbidden_connection(*args, **kwargs):
    raise AssertionError('no control connection permitted in this fixture')

with patch.dict(sys.modules, {'control_http': types.SimpleNamespace(
        connection=forbidden_connection)}):
    import sequence

def wire(value):
    return b'data: ' + json.dumps(value).encode() + b'\n\n'

def completion(text):
    return dict(type='response.completed', response=dict(status='completed',
        output=[dict(content=[dict(type='output_text', text=text)])]))

def event_log(count=0):
    events = Mock()
    events.rows = []
    events.count.side_effect = lambda kind: (1 if kind == 'ai.post.started'
                                           else count if kind == 'ai.post' else 0)
    return events

class StreamEvidenceTests(unittest.TestCase):
    def test_delta_and_terminal_counts_remain_distinct(self):
        for delta in (False, True):
            with self.subTest(delta=delta):
                frames = Frames()
                text = 'fixture text, never retained'
                if delta:
                    frames.feed(wire(dict(type='response.output_text.delta', delta=text)))
                frames.feed(wire(completion(text)))
                frames.finish()
                public = frames.public()
                self.assertEqual(public['delta_text_characters'], len(text) if delta else 0)
                self.assertEqual(public['terminal_text_characters'], len(text))
                self.assertEqual(public['text_characters'], len(text))
                self.assertEqual(public['terminal_completions'], 1)
                self.assertNotIn(text, json.dumps(public))

    def invoke(self, stream, events, report, head=lambda: None):
        with patch.object(sequence, 'send', return_value=stream) as send, \
                patch.object(sequence, 'EventLog', return_value=events):
            try:
                sequence.run_pair('inert', 'inert', None, report,
                                  lambda: None, lambda: None, head)
            finally:
                self.assertEqual(send.call_count, 1)
                events.close.assert_called_once_with()

    def test_completed_call_preserves_evidence_before_cleanup(self):
        before = state(text_characters=9, delta_text_characters=0,
                       terminal_text_characters=9, terminal_completions=1)
        stream = Mock()
        stream.snapshot.side_effect = lambda: dict(before)
        stream.close.side_effect = lambda: before.update(text_characters=0)
        report = dict(post_calls=0, accepted=False)
        with self.assertRaisesRegex(RuntimeError, '^call_finished_before_disconnect$'):
            self.invoke(stream, event_log(1), report)
        for key in ('first_stream_at_check', 'first_stream_before_cleanup'):
            self.assertEqual(report[key]['text_characters'], 9)
            self.assertEqual(report[key]['delta_text_characters'], 0)
        self.assertNotIn('generated_content', report)
        self.assertFalse(report['accepted'])
        stream.close.assert_called_once_with()

    def test_observation_failure_still_saves_first_stream(self):
        stream = Mock(snapshot=Mock(return_value=state(text_characters=7)))
        report = dict(post_calls=0, accepted=False)
        with patch.object(sequence, 'observe', side_effect=RuntimeError('identity changed')):
            with self.assertRaisesRegex(RuntimeError, '^identity changed$'):
                self.invoke(stream, event_log(), report)
        self.assertEqual(report['first_stream_before_cleanup']['text_characters'], 7)
        stream.close.assert_called_once_with()

    def test_failure_after_head_keeps_both_observations(self):
        snapshot = state(text_characters=3)
        stream = Mock(snapshot=Mock(side_effect=lambda: dict(snapshot)))
        events = event_log()
        report = dict(post_calls=0, accepted=False)
        def head():
            snapshot['text_characters'] = 8
            events.count.side_effect = lambda kind: int(kind in ('ai.post.started', 'ai.post'))
        with self.assertRaisesRegex(RuntimeError, '^call_completed_during_head$'):
            self.invoke(stream, events, report, head)
        self.assertEqual(report['first_stream_at_check']['text_characters'], 3)
        self.assertEqual(report['first_stream_at_disconnect_check']['text_characters'], 8)
        self.assertEqual(report['first_stream_before_cleanup']['text_characters'], 8)
        stream.close.assert_called_once_with()

    def test_snapshot_failure_does_not_skip_cleanup(self):
        stream = Mock(snapshot=Mock(side_effect=RuntimeError('snapshot failed')))
        report = dict(post_calls=0, accepted=False)
        with self.assertRaisesRegex(RuntimeError, '^snapshot failed$'):
            self.invoke(stream, event_log(), report)
        stream.close.assert_called_once_with()
        self.assertFalse(report['accepted'])

    def test_cleanup_failure_does_not_erase_evidence(self):
        stream = Mock(snapshot=Mock(return_value=state(text_characters=4)))
        stream.close.side_effect = RuntimeError('cleanup failed')
        report = dict(post_calls=0, accepted=False)
        with self.assertRaisesRegex(RuntimeError, '^cleanup failed$'):
            self.invoke(stream, event_log(1), report)
        self.assertEqual(report['first_stream_before_cleanup']['text_characters'], 4)
        self.assertFalse(report['accepted'])

    def test_success_keeps_original_two_check_sequence(self):
        first, second, events = Mock(), Mock(), event_log()
        first.snapshot.return_value = state(text_characters=6)
        report = dict(post_calls=0, accepted=False)
        with patch.object(sequence, 'send', side_effect=[first, second]) as send, \
                patch.object(sequence, 'EventLog', return_value=events), \
                patch.object(sequence, 'cancel_first', return_value=ROOT) as cancel, \
                patch.object(sequence, 'deliver_second', return_value=OTHER) as deliver:
            sequence.run_pair('inert', 'inert', None, report,
                              lambda: None, lambda: None, lambda: None)
        self.assertEqual(send.call_count, 2)
        self.assertEqual(cancel.call_count, 1)
        self.assertEqual(deliver.call_count, 1)
        self.assertEqual(report['roots'], [ROOT, OTHER])
        self.assertTrue(report['accepted'])
        first.close.assert_called_once_with()
        second.close.assert_called_once_with()
        events.close.assert_called_once_with()
