"""Synthetic frames only; the golden digest is published by castle-rs."""
import base64
import contextlib
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location('v2_token_frame',
    ROOT / 'scripts/experiments/v2_token_frame.py')
FRAME = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(FRAME)
VECTORS = json.loads((Path(__file__).parent / 'data/v2_frame_vectors.json').read_text())


def altered(token, offset, value):
    packet = bytearray(base64.urlsafe_b64decode(token + '=' * (-len(token) % 4)))
    packet[offset] = value ^ packet[0]
    return base64.urlsafe_b64encode(packet).rstrip(b'=').decode()


class FrameTests(unittest.TestCase):
    def refuses(self, token):
        with self.assertRaisesRegex(ValueError, '^token-frame$'):
            FRAME.fingerprint_bytes(token)

    def test_independent_published_golden_frame(self):
        token = VECTORS['golden_token']
        self.assertEqual(hashlib.sha256(token.encode()).hexdigest(),
            '9f43ea555bc670d43fb89839db780c254b31baaee255cdff8e56420f705385ee')
        self.assertEqual(FRAME.fingerprint_bytes(token), bytes.fromhex(VECTORS['golden_inner']))

    def test_five_empty_groups_are_preserved_for_structural_parser(self):
        self.assertEqual(FRAME.fingerprint_bytes(VECTORS['empty_groups_token']),
            bytes.fromhex(VECTORS['empty_groups_inner']))

    def test_input_type_alphabet_size_and_canonical_encoding(self):
        class Text(str):
            pass
        token = VECTORS['golden_token']
        for value in (None, b'x', 42, Text(token), '', 'a' * 16385,
                      token + '=', token + '\n', 'not+base64', token[:-1] + '/'):
            with self.subTest(kind=type(value).__name__):
                self.refuses(value)

    def test_truncation_never_exposes_partial_plaintext(self):
        token = VECTORS['empty_groups_token']
        for length in range(len(token)):
            self.refuses(token[:length])

    def test_version_padding_and_length_marker_are_checked(self):
        token = VECTORS['empty_groups_token']
        for offset, value in ((1, 11), (1, 12), (2, 4), (-1, 0)):
            self.refuses(altered(token, offset, value))

    def test_padding_group_count_and_terminator_refuse(self):
        for name in ('bad_padding', 'wrong_group_count', 'missing_terminator'):
            with self.subTest(name=name):
                self.refuses(VECTORS[name])

    def test_failure_and_success_do_not_print_values(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
            self.refuses('private-input-value')
            FRAME.fingerprint_bytes(VECTORS['golden_token'])
        self.assertEqual(output.getvalue(), '')


if __name__ == '__main__':
    unittest.main()
