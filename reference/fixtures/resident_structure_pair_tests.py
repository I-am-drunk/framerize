"""Inert structural-pair fixtures; no tokens, SDK, account or event-log I/O."""

import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'scripts' / 'experiments'))
from resident_structure_pair import pair_records

LEASE = '12345678-1234-4123-8123-123456789abc'
OTHER_LEASE = '12345678-1234-4123-8123-123456789abd'
PRIVATE = 'synthetic-private-never-exported'


def shape(group=0, field=1, kind=2, width=1):
    return {'entries': [{'group_id': group, 'field_id': field, 'type': kind,
                         'byte_length': width}], 'consumed': 10 + width}


def record(phase, value=None):
    return {'event': 'resident.mint.structure', 'lease': LEASE, 'phase': phase,
            'available': value is not None, 'structure': value}


class Text(str):
    pass


class Mapping(dict):
    pass


class AliasKey(str):
    def __new__(cls, name):
        key = super().__new__(cls, PRIVATE)
        key.name = name
        return key
    def __hash__(self):
        return hash(self.name)
    def __eq__(self, other):
        return other == self.name


class ResidentStructurePairTests(unittest.TestCase):
    def rejected(self, baseline, native):
        with self.assertRaises(ValueError) as caught:
            pair_records(baseline, native)
        self.assertEqual(caught.exception.args, ('resident-structure-pair',))
        self.assertIsNone(caught.exception.__cause__)
        self.assertIsNone(caught.exception.__context__)
        self.assertTrue(caught.exception.__suppress_context__)

    def reject_record(self, index, value):
        pair = [record('baseline', shape()), record('native', shape())]
        pair[index] = value
        self.rejected(*pair)

    def test_two_valid_different_shapes_have_exact_output(self):
        baseline, native = shape(), shape(7, 15, 4, 42)
        actual = pair_records(record('baseline', baseline), record('native', native))
        self.assertEqual(actual, {'same_lease': True,
            'structures': {'baseline': baseline, 'native': native}})
        self.assertIs(actual['same_lease'], True)

    def test_empty_and_unavailable_remain_distinct(self):
        empty = {'entries': [], 'consumed': 10}
        for baseline, native in ((empty, None), (None, empty), (None, None), (empty, empty)):
            result = pair_records(record('baseline', baseline), record('native', native))
            self.assertEqual(result, {'same_lease': True,
                'structures': {'baseline': baseline, 'native': native}})

    def test_records_must_be_plain_dicts(self):
        for index, phase in enumerate(('baseline', 'native')):
            for value in (None, PRIVATE, [], Mapping(record(phase, shape()))):
                with self.subTest(index=index, kind=type(value).__name__):
                    self.reject_record(index, value)

    def test_missing_context_and_logger_envelope_are_rejected(self):
        for index, phase in enumerate(('baseline', 'native')):
            for key in record(phase):
                value = record(phase, shape())
                del value[key]
                self.reject_record(index, value)
            for key in ('token', 'ts', 'request_id', 'level'):
                value = record(phase, shape())
                value[key] = PRIVATE
                self.reject_record(index, value)

    def test_context_values_need_exact_types_and_values(self):
        for index, phase in enumerate(('baseline', 'native')):
            changes = [('event', value) for value in
                       (None, 1, PRIVATE, Text('resident.mint.structure'))]
            changes += [('phase', value) for value in (None, 1, phase.upper(), Text(phase))]
            changes += [('available', value) for value in (None, 0, 1, 'true')]
            for key, replacement in changes:
                value = record(phase, shape())
                value[key] = replacement
                self.reject_record(index, value)

    def test_lease_must_be_canonical_nonnil_plain_uuid(self):
        bad = (None, 1, Text(LEASE), '', LEASE.upper(), LEASE.replace('-', ''),
               '{' + LEASE + '}', 'urn:uuid:' + LEASE, LEASE + '\n', 'x' * 36,
               '00000000-0000-0000-0000-000000000000')
        for index, phase in enumerate(('baseline', 'native')):
            for lease in bad:
                value = record(phase, shape())
                value['lease'] = lease
                self.reject_record(index, value)

    def test_different_leases_refuse_even_when_unavailable(self):
        for observed in (None, shape()):
            baseline, native = record('baseline', observed), record('native', observed)
            native['lease'] = OTHER_LEASE
            self.rejected(baseline, native)

    def test_phases_are_position_specific(self):
        baseline, native = record('baseline', shape()), record('native', shape())
        self.rejected(native, baseline)
        self.rejected(baseline, baseline)
        self.rejected(native, native)

    def test_availability_requires_matching_structure(self):
        for index, phase in enumerate(('baseline', 'native')):
            for observed, available in ((None, True), (shape(), False),
                                        ({'entries': [], 'consumed': 10}, False)):
                value = record(phase, observed)
                value['available'] = available
                self.reject_record(index, value)

    def test_shape_container_and_extra_fields_are_rejected(self):
        bad = [PRIVATE, [], {}, Mapping(shape()), {**shape(), 'token': PRIVATE}]
        extra = shape()
        extra['entries'][0]['value'] = PRIVATE
        bad.append(extra)
        for index, phase in enumerate(('baseline', 'native')):
            for observed in bad:
                self.reject_record(index, record(phase, observed))

    def test_qualified_field_and_length_validation_is_retained(self):
        bad = [shape(5), shape(field=32), shape(kind=8), shape(width=0), shape(kind=True)]
        bad += [{'entries': [], 'consumed': 11}, {'entries': [], 'consumed': True},
                {'entries': (), 'consumed': 10}, {'entries': [Mapping(shape()['entries'][0])],
                                                'consumed': 11}]
        row = shape()['entries'][0]
        bad.append({'entries': [row, row], 'consumed': 12})
        for index, phase in enumerate(('baseline', 'native')):
            for observed in bad:
                self.reject_record(index, record(phase, observed))

    def test_alias_keys_are_refused_at_every_level(self):
        layers = [('record', ('event', 'lease', 'phase', 'available', 'structure')),
                  ('shape', ('entries', 'consumed')),
                  ('entry', ('group_id', 'field_id', 'type', 'byte_length'))]
        for index, phase in enumerate(('baseline', 'native')):
            for layer, keys in layers:
                for key in keys:
                    for key_type in (Text, AliasKey):
                        value = record(phase, shape())
                        target = {'record': value, 'shape': value['structure'],
                                  'entry': value['structure']['entries'][0]}[layer]
                        original = target.pop(key)
                        target[key_type(key)] = original
                        with self.subTest(index=index, layer=layer, key=key, alias=key_type):
                            self.reject_record(index, value)

    def test_ordinary_projection_failures_have_no_private_error_chain(self):
        with patch('resident_structure_pair._structure', side_effect=ValueError(PRIVATE)):
            self.rejected(record('baseline', shape()), record('native', shape()))

    def test_nested_copies_are_independent_even_for_shared_input(self):
        shared = shape()
        result = pair_records(record('baseline', shared), record('native', shared))
        baseline, native = result['structures']['baseline'], result['structures']['native']
        for copy in (baseline, native):
            self.assertIsNot(copy, shared)
            self.assertIsNot(copy['entries'], shared['entries'])
            self.assertIsNot(copy['entries'][0], shared['entries'][0])
        self.assertIsNot(baseline['entries'][0], native['entries'][0])
        baseline['entries'][0]['type'] = 1
        self.assertEqual(native['entries'][0]['type'], 2)
        self.assertEqual(shared['entries'][0]['type'], 2)
        shared['entries'][0]['byte_length'] = 9
        shared['consumed'] = 19
        self.assertEqual(native, shape())
        self.assertEqual(baseline['entries'][0]['byte_length'], 1)
        self.assertEqual(baseline['consumed'], 11)

    def test_output_has_plain_literal_keys_and_no_input_context(self):
        result = pair_records(record('baseline', shape()), record('native', shape()))
        encoded = json.dumps(result)
        for private in (LEASE, PRIVATE, 'resident.mint.structure', 'event', 'phase', 'available'):
            self.assertNotIn(private, encoded)
        pending = [result]
        while pending:
            value = pending.pop()
            if type(value) is dict:
                self.assertTrue(all(type(key) is str for key in value))
                pending.extend(value.values())
            elif type(value) is list:
                pending.extend(value)
            else:
                self.assertIn(type(value), (int, bool, type(None)))


if __name__ == '__main__':
    unittest.main()
