"""Synthetic resident observation fixtures; no minter, SDK or account data."""

import copy
import gc
import json
from pathlib import Path
import sys
import unittest
import weakref

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'scripts' / 'experiments'))
from resident_structure_observer import record

LEASE = '12345678-1234-4123-8123-123456789abc'
TOKEN = 'synthetic-private-token-never-export'


def shape():
    return {'entries': [{'group_id': 0, 'field_id': 1, 'type': 2,
                         'byte_length': 3}], 'consumed': 13}


def event(structure=None, phase='baseline'):
    return {'event': 'resident.mint.structure', 'lease': LEASE, 'phase': phase,
            'available': structure is not None, 'structure': structure}


class ResidentStructureObserverTests(unittest.TestCase):
    def observe(self, value):
        events, seen = [], []
        def describe(token):
            seen.append(token)
            return value
        result = record(TOKEN, LEASE, 'baseline', describe, events.append)
        self.assertIsNone(result)
        self.assertEqual(seen, [TOKEN])
        self.assertEqual(len(events), 1)
        self.assertNotIn(TOKEN, json.dumps(events))
        return events[0]

    def test_emit_failure_is_swallowed_without_retry(self):
        for value in (shape(), None):
            emitted, calls = [], []
            def describe(token):
                calls.append(token)
                return value
            def emit(facts):
                emitted.append(facts)
                raise RuntimeError(TOKEN)
            self.assertIsNone(record(TOKEN, LEASE, 'baseline', describe, emit))
            self.assertEqual(calls, [TOKEN])
            self.assertEqual(emitted, [event(value)])
            self.assertNotIn(TOKEN, json.dumps(emitted))

    def test_exact_fixed_output_for_both_phases(self):
        for phase in ('baseline', 'native'):
            events, seen = [], []
            def describe(token):
                seen.append(token)
                return shape()
            self.assertIsNone(record(TOKEN, LEASE, phase, describe, events.append))
            self.assertEqual(events, [event(shape(), phase)])
            self.assertEqual(len(seen), 1)
            self.assertIs(seen[0], TOKEN)
            self.assertIs(events[0]['available'], True)
            self.assertNotIn(TOKEN, json.dumps(events))

    def test_invalid_context_invokes_neither_port(self):
        class Text(str):
            pass
        leases = (None, True, 1, {}, Text(LEASE), '', LEASE.upper(),
                  LEASE.replace('-', ''), '{' + LEASE + '}', 'urn:uuid:' + LEASE,
                  ' ' + LEASE, LEASE + '\n', 'x' * 36, 'x' * 10000,
                  '00000000-0000-0000-0000-000000000000')
        phases = (None, True, 1, {}, Text('baseline'), '', 'Baseline',
                  'native\n', 'mint', TOKEN)
        contexts = [(lease, 'baseline') for lease in leases]
        contexts += [(LEASE, phase) for phase in phases]
        for lease, phase in contexts:
            calls = []
            self.assertIsNone(record(TOKEN, lease, phase, calls.append, calls.append))
            self.assertEqual(calls, [])

    def test_valid_empty_structure_remains_available(self):
        empty = {'entries': [], 'consumed': 10}
        actual = self.observe(empty)
        self.assertEqual(actual, event(empty))
        self.assertIs(actual['available'], True)

    def test_extra_fields_are_rejected_not_forwarded(self):
        for key in ('token', 'value', 'hash', 'error', 'events'):
            for nested in (False, True):
                value = shape()
                target = value['entries'][0] if nested else value
                target[key] = TOKEN
                actual = self.observe(value)
                self.assertEqual(actual, event())
                self.assertIs(actual['available'], False)

    def test_malformed_structure_types_are_unavailable(self):
        class Mapping(dict):
            pass
        values = [None, TOKEN, [], (shape(),), Mapping(shape()),
                  {'entries': (), 'consumed': 10},
                  {'entries': [Mapping(shape()['entries'][0])], 'consumed': 13},
                  {'entries': [TOKEN], 'consumed': 13}]
        for consumed in (True, 13.0, '13', None):
            value = shape()
            value['consumed'] = consumed
            values.append(value)
        for field in ('group_id', 'field_id', 'type', 'byte_length'):
            for bad in (True, 1.0, TOKEN, None):
                value = shape()
                value['entries'][0][field] = bad
                values.append(value)
        for value in values:
            self.assertEqual(self.observe(value), event())

    def test_structural_bounds_accounting_and_uniqueness_are_enforced(self):
        values = [{'entries': [], 'consumed': 11},
                  {'entries': [], 'consumed': 16385},
                  {'entries': shape()['entries'] * 156, 'consumed': 478}]
        for field, bad in (('group_id', 1), ('field_id', -1), ('field_id', 32),
                           ('type', 8), ('byte_length', 0), ('byte_length', 16385)):
            value = shape()
            value['entries'][0][field] = bad
            values.append(value)
        duplicate = shape()
        duplicate['entries'] *= 2
        duplicate['consumed'] = 16
        values.append(duplicate)
        reverse = shape()
        reverse['entries'].insert(0, dict(group_id=9, field_id=0, type=0, byte_length=1))
        reverse['consumed'] = 14
        for value in values + [reverse]:
            self.assertEqual(self.observe(value), event())

    def test_describe_failure_emits_unavailable_once_without_details(self):
        calls, events = [], []
        def describe(token):
            calls.append(token)
            raise RuntimeError(TOKEN)
        self.assertIsNone(record(TOKEN, LEASE, 'native', describe, events.append))
        self.assertEqual(calls, [TOKEN])
        self.assertEqual(events, [event(phase='native')])
        self.assertIs(events[0]['available'], False)
        self.assertNotIn(TOKEN, json.dumps(events))

    def test_structure_is_copied_before_emission(self):
        source = shape()
        snapshot = copy.deepcopy(source)
        observed = self.observe(source)['structure']
        self.assertIsNot(observed, source)
        self.assertIsNot(observed['entries'], source['entries'])
        self.assertIsNot(observed['entries'][0], source['entries'][0])
        source['entries'][0]['field_id'] = 2
        source['entries'].append({'token': TOKEN})
        source['consumed'] = 99
        self.assertEqual(observed, snapshot)

    def test_emit_mutation_cannot_change_decoder_structure(self):
        source, calls = shape(), []
        def emit(facts):
            calls.append(True)
            facts['structure']['entries'][0]['field_id'] = 2
            facts['structure']['entries'].clear()
            facts['structure']['consumed'] = 99
            raise RuntimeError(TOKEN)
        self.assertIsNone(record(TOKEN, LEASE, 'baseline', lambda token: source, emit))
        self.assertEqual(calls, [True])
        self.assertEqual(source, shape())

    def test_uncallable_ports_are_contained(self):
        events = []
        self.assertIsNone(record(TOKEN, LEASE, 'baseline', None, events.append))
        self.assertEqual(events, [event()])
        calls = []
        def describe(token):
            calls.append(token)
            return shape()
        self.assertIsNone(record(TOKEN, LEASE, 'baseline', describe, None))
        self.assertEqual(calls, [TOKEN])

    def test_token_and_callbacks_are_not_retained(self):
        class Token:
            pass
        for fail in (False, True):
            token = Token()
            def describe(value):
                if fail:
                    raise RuntimeError(value)
                return shape()
            def emit(facts):
                pass
            refs = [weakref.ref(value) for value in (token, describe, emit)]
            self.assertIsNone(record(token, LEASE, 'native', describe, emit))
            del token, describe, emit
            gc.collect()
            self.assertTrue(all(ref() is None for ref in refs))

    def test_control_flow_exceptions_are_not_swallowed(self):
        class Stop(BaseException):
            pass
        def stop(value):
            raise Stop()
        for describe, emit in ((stop, lambda facts: None),
                               (lambda token: shape(), stop)):
            with self.assertRaises(Stop):
                record(TOKEN, LEASE, 'baseline', describe, emit)


if __name__ == '__main__':
    unittest.main()
