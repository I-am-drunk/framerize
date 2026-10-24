"""Synthetic event framing; never reads logs, accounts or SDK output."""
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'scripts/experiments'))
from resident_structure_json import decode_record
from resident_structure_pair import pair_records

LEASE = '00000000-0000-0000-0000-000000000001'


def record(phase):
    return {'event': 'resident.mint.structure', 'lease': LEASE, 'phase': phase,
            'available': False, 'structure': None}


class StructureJsonTests(unittest.TestCase):
    def refused(self, raw):
        try:
            decode_record(raw)
        except ValueError as error:
            self.assertEqual(str(error), 'resident-structure-json')
            self.assertIsNone(error.__context__)
            self.assertIsNone(error.__cause__)
        else:
            self.fail('malformed record accepted')

    def test_genuine_records_remain_pairable(self):
        before = record('baseline')
        after = record('native')
        parsed = [decode_record(json.dumps(v).encode()) for v in (before, after)]
        self.assertEqual(parsed, [before, after])
        self.assertEqual(pair_records(*parsed), pair_records(before, after))

    def test_duplicate_envelope_and_context_keys_refuse(self):
        valid = json.dumps(record('native'))
        for key in ('event', 'lease', 'phase', 'available', 'structure'):
            with self.subTest(key=key):
                raw = '{' + json.dumps(key) + ':"discarded-private-fixture",' + valid[1:]
                self.refused(raw.encode())

    def test_duplicate_nested_fields_refuse(self):
        for raw in (
            b'{"structure":{"consumed":99,"consumed":10,"entries":[]}}',
            b'{"structure":{"entries":[{"field_id":1,"field_id":2}]}}',
            b'{"structure":{"entries":[{"type":1,"type":2}]}}',
            b'{"structure":{"entries":[{"byte_length":2,"byte_length":3}]}}',
        ):
            self.refused(raw)

    def test_escaped_and_identical_duplicates_refuse(self):
        self.refused(b'{"phase":"native","\\u0070hase":"native"}')
        self.refused(b'{"lease":"same","lease":"same"}')

    def test_bad_encoding_json_and_nonobjects_have_fixed_errors(self):
        for raw in (b'', b'{"private-fixture":', b'\xff', b'[]', b'null',
                    b'{"x":1}{"x":2}', 'not-bytes', bytearray(b'{}')):
            with self.subTest(kind=type(raw).__name__):
                self.refused(raw)

    def test_separate_objects_may_reuse_names(self):
        raw = b'{"entries":[{"type":1},{"type":2}],"text":"Unicode \\u00e9"}'
        self.assertEqual(decode_record(raw),
                         {'entries': [{'type': 1}, {'type': 2}], 'text': 'Unicode é'})


if __name__ == '__main__':
    unittest.main()
