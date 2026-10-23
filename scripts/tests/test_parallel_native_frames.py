import json
import unittest
from native_check_fixture import event, terminal
from parallel_native_check.frames import Frames
from parallel_native_check.support import Refusal


class FramesTests(unittest.TestCase):
    def test_fragmented_unicode_stream_keeps_counts_without_content(self):
        frames = Frames()
        wire = event('response.output_text.delta', delta='private-\u2603') + terminal('private-\u2603')
        for byte in wire + b'data: [DONE]\n\n':
            frames.feed(bytes([byte]))
        frames.finish()
        self.assertEqual(frames.public()['text_characters'], 9)
        self.assertNotIn('private', json.dumps(frames.public()))

    def test_terminal_output_can_supply_text_without_deltas(self):
        frames = Frames()
        frames.feed(terminal())
        frames.finish()
        self.assertEqual(frames.public()['text_characters'], 2)

    def test_open_frames_are_not_completion(self):
        frames = Frames()
        frames.feed(event('response.created', response={}))
        with self.assertRaisesRegex(Refusal, 'missing_terminal_completion'):
            frames.finish()

    def test_provider_error_frames_are_rejected_without_their_text(self):
        for kind in ('error', 'response.failed', 'response.incomplete'):
            with self.subTest(kind=kind), self.assertRaisesRegex(Refusal, '^stream_error$'):
                Frames().feed(event(kind, error={'message': 'private-provider-text'}))

    def test_trailing_events_and_duplicate_done_are_refused(self):
        for extra in (terminal(), event('response.output_text.delta', delta='late'),
                      b'data: [DONE]\n\ndata: [DONE]\n\n'):
            frames = Frames()
            frames.feed(terminal())
            with self.subTest(extra=extra[:20]), self.assertRaises(Refusal):
                frames.feed(extra)

    def test_partial_frame_and_byte_overflow_are_refused(self):
        frames = Frames()
        frames.feed(terminal() + b'data: {')
        with self.assertRaisesRegex(Refusal, 'partial_stream_frame'):
            frames.finish()
        with self.assertRaisesRegex(Refusal, 'stream_byte_limit'):
            Frames().feed(b'x' * 65537)

    def test_empty_or_inconsistent_text_is_not_delivery(self):
        for delta, text in (('', ''), ('partial', 'partial-extra')):
            frames = Frames()
            frames.feed(event('response.output_text.delta', delta=delta) + terminal(text))
            with self.subTest(delta=delta), self.assertRaises(Refusal):
                frames.finish()

    def test_event_label_must_match_payload_type(self):
        wire = terminal().replace(b'event: response.completed', b'event: response.failed')
        with self.assertRaisesRegex(Refusal, 'stream_event_type_mismatch'):
            Frames().feed(wire)
