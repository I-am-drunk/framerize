import unittest
from evidence import Evidence, root_id
from criteria import ready_to_disconnect, complete, final_counts

ROOT = '11111111-1111-4111-8111-111111111111'
CHILD = '22222222-2222-4222-8222-222222222222'
OTHER = '33333333-3333-4333-8333-333333333333'

def row(event, operation, parent=None, confirmed=None):
    return dict(event=event, operation_id=operation, request_id=ROOT,
                parent_operation_id=parent, cleanup_confirmed=confirmed,
                ts='2026-10-24T22:00:01Z')

def evidence():
    result = Evidence()
    result.add(row('operation.registered', ROOT))
    result.add(row('operation.registered', CHILD, ROOT))
    return result

def finish(result):
    result.add(row('operation.joined', CHILD, ROOT, True))
    result.add(row('operation.released', CHILD, ROOT, True))
    result.add(row('operation.released', ROOT, confirmed=True))

def state(**changes):
    return dict(status=200, operation_id=ROOT, finished=False, eof=False,
                read_error=None, terminal_completions=0, text_characters=0,
                events=4) | changes

class AcceptanceTests(unittest.TestCase):
    def test_opening_frames_are_not_generated_content(self):
        self.assertFalse(ready_to_disconnect(state()))
        self.assertTrue(ready_to_disconnect(state(text_characters=1)))

    def test_finished_stream_is_not_a_disconnect_test(self):
        with self.assertRaisesRegex(RuntimeError, 'finished_before_disconnect'):
            ready_to_disconnect(state(text_characters=1, finished=True))

    def test_missing_or_invalid_header_refuses(self):
        for value in (None, '', 'private-string', '00000000-0000-0000-0000-000000000000'):
            with self.subTest(value=value), self.assertRaises(RuntimeError):
                root_id(value)

    def test_join_and_child_release_do_not_replace_root_release(self):
        result = evidence()
        result.add(row('operation.joined', CHILD, ROOT, True))
        self.assertFalse(result.joined(ROOT))
        result.add(row('operation.released', CHILD, ROOT, True))
        self.assertFalse(result.joined(ROOT))
        result.add(row('operation.released', ROOT, confirmed=True))
        self.assertTrue(result.joined(ROOT))

    def test_other_request_cannot_supply_missing_release(self):
        result = evidence()
        finish(result)
        result.rows[-1]['request_id'] = OTHER
        self.assertFalse(result.joined(ROOT))

    def test_uncertain_join_and_duplicate_events_refuse(self):
        for defect in ('uncertain', 'duplicate', 'order'):
            result = evidence()
            finish(result)
            if defect == 'uncertain': result.rows[2]['cleanup_confirmed'] = False
            if defect == 'duplicate': result.add(result.rows[-1])
            if defect == 'order': result.rows[2], result.rows[3] = result.rows[3], result.rows[2]
            with self.subTest(defect=defect), self.assertRaises(RuntimeError):
                result.joined(ROOT)

    def test_completion_before_disconnect_is_rejected(self):
        import datetime
        result = evidence()
        finish(result)
        at = datetime.datetime.fromisoformat('2026-10-24T22:00:02+00:00').timestamp()
        with self.assertRaisesRegex(RuntimeError, 'predates_disconnect'):
            result.joined(ROOT, after=at)

    def test_completed_response_requires_eof_text_and_terminal(self):
        good = state(finished=True, eof=True, text_characters=2, terminal_completions=1)
        self.assertTrue(complete(good))
        for field, value in (('eof', False), ('text_characters', 0), ('terminal_completions', 0)):
            with self.subTest(field=field), self.assertRaises(RuntimeError):
                complete(good | {field: value})

    def test_extra_inference_or_wrong_chain_identity_refuses(self):
        result = Evidence()
        for kind in ('ai.post.started', 'ai.post', 'ai.chain'):
            result.add(dict(event=kind, request_id=ROOT))
        final_counts(result, 1, [ROOT])
        with self.assertRaises(RuntimeError):
            final_counts(result, 1, [OTHER])
        result.add(dict(event='ai.post.started'))
        with self.assertRaises(RuntimeError):
            final_counts(result, 1, [ROOT])

    def test_event_report_excludes_credentials_and_response_text(self):
        result = Evidence()
        result.add(dict(event='ai.chain', request_id=ROOT, cookie='secret',
                        reason='private response', body='private text'))
        self.assertEqual(result.rows, [dict(event='ai.chain', request_id=ROOT)])
