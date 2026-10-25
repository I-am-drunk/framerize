import unittest
from unittest.mock import Mock, patch
from test_stream_evidence import sequence, event_log, ROOT, OTHER, state
from criteria import ready_for_started_request, ready_to_disconnect

class InterruptionBoundaryTests(unittest.TestCase):
    def fixture(self):
        current = state(text_characters=0, delta_text_characters=0)
        stream = Mock(snapshot=Mock(side_effect=lambda: dict(current)))
        events = event_log()
        events.joined.return_value = True
        def close(*, cancel=False):
            self.assertTrue(cancel)
            current['disconnected_at_unix'] = 1.0
            events.count.side_effect = lambda kind: int(kind in
                ('ai.post.started', 'ai.post', 'ai.chain'))
            events.rows = [{'event': 'ai.chain', 'request_id': ROOT}]
        stream.close.side_effect = close
        return current, stream, events

    def test_request_started_does_not_require_buffered_response_text(self):
        current, stream, events = self.fixture()
        report = {}
        root = sequence.cancel_first(stream, events, report, lambda: None,
                                     lambda: None, boundary='request_started')
        self.assertEqual(root, ROOT)
        stream.close.assert_called_once_with(cancel=True)
        self.assertEqual(report['interruption_boundary'], 'request_started')
        self.assertEqual(report['interruption_ready_state']['text_characters'], 0)
        self.assertNotIn('generated_content', report)
        self.assertIn('joined_cleanup_at_unix', report)
        self.assertIs(report['upstream_cancellation_proven'], False)
        self.assertIs(report['text_observed_before_disconnect'], False)

    def test_both_boundaries_keep_stream_validity_guards(self):
        bad = [dict(read_error='broken'), dict(finished=True), dict(eof=True),
               dict(terminal_completions=1), dict(status=500), dict(operation_id='bad')]
        for check in (ready_for_started_request, ready_to_disconnect):
            self.assertFalse(check(state(status=None)))
            for change in bad:
                with self.subTest(check=check.__name__, change=change):
                    with self.assertRaises(RuntimeError):
                        check(state(text_characters=1, **change))
        self.assertTrue(ready_for_started_request(state()))
        self.assertFalse(ready_to_disconnect(state()))

    def test_completed_upstream_still_refuses_in_both_modes(self):
        for boundary in ('generated_text', 'request_started'):
            current, stream, events = self.fixture()
            events.count.side_effect = lambda kind: int(kind in ('ai.post.started', 'ai.post'))
            with self.subTest(boundary=boundary):
                with self.assertRaisesRegex(RuntimeError, '^call_finished_before_disconnect$'):
                    sequence.cancel_first(stream, events, {}, lambda: None,
                                          lambda: None, boundary=boundary)
                stream.close.assert_not_called()

    def test_unknown_mode_refuses_before_sending(self):
        for boundary in (None, [], 'guess', 'active_upstream'):
            with self.subTest(boundary=boundary), patch.object(sequence, 'send') as send:
                with self.assertRaisesRegex(RuntimeError, '^unknown_interruption_boundary$'):
                    sequence.run_pair('inert', 'inert', None, {'post_calls': 0},
                        lambda: None, lambda: None, lambda: None,
                        cancellation_boundary=boundary)
                send.assert_not_called()

    def test_upstream_completion_during_head_prevents_disconnect(self):
        current, stream, events = self.fixture()
        def head():
            events.count.side_effect = lambda kind: int(kind in ('ai.post.started', 'ai.post'))
        with self.assertRaisesRegex(RuntimeError, '^call_completed_during_head$'):
            sequence.cancel_first(stream, events, {}, lambda: None, head,
                                  boundary='request_started')
        stream.close.assert_not_called()

    def test_started_mode_requires_real_delta_on_second_delivery(self):
        for delta in (0, 7):
            with self.subTest(delta=delta):
                first, second, events = Mock(), Mock(), event_log()
                first.snapshot.return_value = state()
                report = dict(post_calls=0, accepted=False)
                def deliver(*args):
                    report['completed_stream'] = state(delta_text_characters=delta,
                        terminal_text_characters=7, text_characters=7)
                    return OTHER
                with patch.object(sequence, 'send', side_effect=[first, second]), \
                        patch.object(sequence, 'EventLog', return_value=events), \
                        patch.object(sequence, 'cancel_first', return_value=ROOT), \
                        patch.object(sequence, 'deliver_second', side_effect=deliver):
                    if delta:
                        sequence.run_pair('inert', 'inert', None, report,
                            lambda: None, lambda: None, lambda: None,
                            cancellation_boundary='request_started')
                        self.assertTrue(report['accepted'])
                        self.assertEqual(report['acceptance_scope'],
                            'request_started_disconnect_then_delivery')
                    else:
                        with self.assertRaisesRegex(RuntimeError, '^delivery_delta_missing$'):
                            sequence.run_pair('inert', 'inert', None, report,
                                lambda: None, lambda: None, lambda: None,
                                cancellation_boundary='request_started')
                        self.assertFalse(report['accepted'])
                first.close.assert_called_once_with()
                second.close.assert_called_once_with()
                events.close.assert_called_once_with()
