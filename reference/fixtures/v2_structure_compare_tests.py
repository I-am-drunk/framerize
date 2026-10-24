"""Synthetic comparison fixtures. No SDK, account, token or network input."""

import copy
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'scripts' / 'experiments'))
from v2_structure_compare import compare_structures as compare
from v2_token_structure import parse_fingerprint_section


def shape(*entries):
    rows = [dict(zip(('group_id', 'field_id', 'type', 'byte_length'), row))
            for row in entries]
    return {'entries': rows, 'consumed': 10 + sum(row[3] for row in entries)}


def parsed(group_id, field_id, kind, payload=b''):
    field = bytes([(field_id << 3) | kind]) + payload
    data = b''.join(((g << 5) | int(g == group_id)).to_bytes(2, 'big')
                    + (field if g == group_id else b'') for g in (0, 4, 7, 8, 9))
    entries, consumed = parse_fingerprint_section(data)
    return {'entries': entries, 'consumed': consumed}


class StructureComparisonTests(unittest.TestCase):
    def test_empty_available_observations_match(self):
        self.assertEqual(compare(shape(), shape()), {
            'comparable': True, 'structural_match': True,
            'baseline_consumed': 10, 'native_consumed': 10, 'differences': [],
        })

    def test_unavailable_is_not_an_empty_or_matching_observation(self):
        for left, right in ((None, None), (None, shape()), (shape(), None)):
            self.assertEqual(compare(left, right), {
                'comparable': False, 'structural_match': None, 'differences': [],
                'baseline_consumed': None if left is None else 10,
                'native_consumed': None if right is None else 10,
            })

    def test_boolean_tags_differ_without_payload_or_length_change(self):
        result = compare(parsed(0, 1, 1), parsed(0, 1, 2))
        self.assertIs(result['structural_match'], False)
        self.assertEqual(result['baseline_consumed'], result['native_consumed'])
        self.assertEqual(result['differences'], [{
            'group_id': 0, 'field_id': 1,
            'baseline': {'type': 1, 'byte_length': 1},
            'native': {'type': 2, 'byte_length': 1},
        }])

    def test_equal_totals_do_not_hide_field_length_changes(self):
        result = compare(shape((0, 1, 4, 4), (0, 2, 4, 6)),
                         shape((0, 1, 4, 6), (0, 2, 4, 4)))
        self.assertEqual(result['baseline_consumed'], 20)
        self.assertEqual(result['native_consumed'], 20)
        self.assertIs(result['structural_match'], False)
        self.assertEqual([row['field_id'] for row in result['differences']], [1, 2])

    def test_presence_is_not_conflated_with_an_unchanged_count(self):
        result = compare(shape((0, 1, 1, 1)), shape((0, 2, 1, 1)))
        self.assertIs(result['structural_match'], False)
        first, second = result['differences']
        self.assertEqual((first['field_id'], second['field_id']), (1, 2))
        self.assertIsNone(first['native'])
        self.assertIsNone(second['baseline'])

    def test_groups_keep_identical_field_ids_distinct(self):
        result = compare(shape((0, 1, 1, 1), (9, 1, 1, 1)),
                         shape((0, 1, 1, 1), (4, 1, 2, 1)))
        self.assertEqual([row['group_id'] for row in result['differences']], [4, 9])

    def test_same_width_distinct_payloads_are_indistinguishable(self):
        cases = ((0, 1, 3, b'\x01', b'\xff'),
                 (0, 1, 4, b'\x03cat', b'\x03dog'),
                 (0, 1, 5, b'\x01', b'\x7f'),
                 (0, 1, 6, b'\x01', b'\xff'),
                 (0, 9, 7, bytes(5), b'\xff' * 5),
                 (7, 12, 7, b'\x09' + bytes(54), b'\x09' + b'\xff' * 54))
        for g, f, kind, left, right in cases:
            with self.subTest(group=g, field=f, kind=kind):
                result = compare(parsed(g, f, kind, left), parsed(g, f, kind, right))
                self.assertIs(result['structural_match'], True)
                self.assertEqual(result['differences'], [])

    def test_bitset_length_hides_count_and_bits_within_same_bucket(self):
        left = parsed(0, 11, 7, b'\x01\x00')
        right = parsed(0, 11, 7, b'\x08\xff')
        self.assertIs(compare(left, right)['structural_match'], True)
        wider = parsed(0, 11, 7, b'\x09\x00\x00')
        self.assertIs(compare(right, wider)['structural_match'], False)

    def test_dimension_length_hides_which_pair_is_compact(self):
        compact, separate = b'\x80\x01', b'\x00\x02\x00\x03'
        left = parsed(0, 4, 7, compact + separate)
        right = parsed(0, 4, 7, separate + compact)
        self.assertIs(compare(left, right)['structural_match'], True)

    def test_packed_integer_boundary_is_visible_not_its_value(self):
        small = parsed(0, 1, 5, b'\x7f')
        large = parsed(0, 1, 5, b'\x80\x80')
        self.assertIs(compare(small, large)['structural_match'], False)

    def test_output_is_detached_from_input_structures(self):
        left, right = shape((0, 1, 1, 1)), shape((0, 1, 2, 1))
        saved = copy.deepcopy((left, right))
        result = compare(left, right)
        self.assertEqual((left, right), saved)
        result['differences'][0]['baseline']['type'] = 7
        result['differences'].clear()
        self.assertEqual((left, right), saved)

    def test_reordered_projection_entries_are_compared_by_identity(self):
        left = shape((0, 1, 1, 1), (0, 2, 2, 1))
        right = shape((0, 2, 2, 1), (0, 1, 1, 1))
        self.assertIs(compare(left, right)['structural_match'], True)
        # This compares projection metadata, not parser authenticity.

    def test_malformed_input_has_a_fixed_error_even_if_other_side_unavailable(self):
        bad = [False, [], 'private-marker', {'entries': [], 'consumed': 11},
               shape((0, 1, True, 1)), shape((0, 1, 1, 1), (0, 1, 2, 1))]
        extra = shape((0, 1, 1, 1))
        extra['entries'][0]['private-marker'] = 'private-marker'
        bad.append(extra)
        for value in bad:
            for left, right in ((value, None), (None, value)):
                with self.assertRaises(ValueError) as caught:
                    compare(left, right)
                self.assertEqual(caught.exception.args, ('structure-comparison',))
                self.assertTrue(caught.exception.__suppress_context__)
                self.assertIsNone(caught.exception.__context__)
                self.assertIsNone(caught.exception.__cause__)

    def test_validator_error_context_and_cause_are_detached(self):
        original = RuntimeError('synthetic-private-context')
        with patch('v2_structure_compare._structure', side_effect=original):
            with self.assertRaises(ValueError) as caught:
                compare(shape(), None)
        self.assertEqual(caught.exception.args, ('structure-comparison',))
        self.assertIsNone(caught.exception.__context__)
        self.assertIsNone(caught.exception.__cause__)
        self.assertTrue(caught.exception.__suppress_context__)

    def test_full_five_group_bound_is_supported(self):
        value = shape(*((g, f, 1, 1) for g in (0, 4, 7, 8, 9) for f in range(31)))
        self.assertIs(compare(value, value)['structural_match'], True)

    def test_emitted_keys_are_canonical_not_copied_from_input(self):
        class AliasedKey(str):
            def __hash__(self):
                return hash('group_id')

            def __eq__(self, other):
                return other == 'group_id'

        left = shape((0, 1, 1, 1))
        row = left['entries'][0]
        row[AliasedKey('private-marker')] = row.pop('group_id')
        result = compare(left, shape((0, 1, 2, 1)))
        self.assertNotIn('private-marker', json.dumps(result))
        self.assertEqual(result['differences'][0]['group_id'], 0)
        self.assertTrue(all(type(key) is str for key in result['differences'][0]))

    def test_reported_four_field_shapes_keep_only_structural_facts(self):
        # Hand-built shape subset, not captured tokens or a matched runtime pair.
        synthetic = shape((7, 13, 2, 1), (7, 15, 4, 42), (7, 23, 7, 7))
        browser = shape((7, 12, 7, 56), (7, 13, 1, 1), (7, 15, 3, 2))
        result = compare(browser, synthetic)
        self.assertIs(result['structural_match'], False)
        self.assertEqual([row['field_id'] for row in result['differences']],
                         [12, 13, 15, 23])
        for row in result['differences']:
            self.assertEqual(set(row), {'group_id', 'field_id', 'baseline', 'native'})
            for side in ('baseline', 'native'):
                if row[side] is not None:
                    self.assertEqual(set(row[side]), {'type', 'byte_length'})


if __name__ == '__main__':
    unittest.main()
