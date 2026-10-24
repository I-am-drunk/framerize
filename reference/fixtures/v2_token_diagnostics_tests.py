"""Synthetic composition checks for the private diagnostic pipeline."""
import importlib.util
from pathlib import Path
import unittest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('projection_fixtures', HERE / 'v2_token_projection_tests.py')
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)
STRUCTURE = fixtures.load('v2_token_structure')

def describe(token):
    data = fixtures.FRAME.fingerprint_bytes(token)
    entries, consumed = STRUCTURE.parse_fingerprint_section(data)
    if consumed >= len(data):
        raise ValueError('token-structure')
    return {'entries': entries, 'consumed': consumed}

class DiagnosticTests(unittest.TestCase):
    def test_real_frame_parser_projection_composition_omits_trailer(self):
        value = fixtures.report()
        raw = fixtures.raw(value)
        result = fixtures.PROJECTION.project_report(raw, describe)
        self.assertEqual(raw, b'')
        self.assertEqual(result['structure'], {'entries': [], 'consumed': 10})
        inner = fixtures.FRAME.fingerprint_bytes(value['_private_token'])
        self.assertEqual(inner[result['structure']['consumed']:], b'\x00\x00\xff')
        self.assertNotIn(value['_private_token'], repr(result))

    def test_invalid_frames_fail_before_publishing_any_structure(self):
        for name in ('wrong_group_count', 'bad_padding', 'missing_terminator'):
            with self.subTest(vector=name):
                value = fixtures.report(); value['_private_token'] = fixtures.VECTORS[name]
                raw = fixtures.raw(value)
                with self.assertRaisesRegex(ValueError, '^token-projection$'):
                    fixtures.PROJECTION.project_report(raw, describe)
                self.assertEqual(raw, b'')

if __name__ == '__main__':
    unittest.main()
