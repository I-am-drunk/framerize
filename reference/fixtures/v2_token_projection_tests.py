"""Inert report projection tests. No SDK or account data."""
import copy
import importlib.util
import json
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / f'scripts/experiments/{name}.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module
PROJECTION = load('v2_token_projection')
FRAME = load('v2_token_frame')
VECTORS = json.loads((Path(__file__).parent / 'data/v2_frame_vectors.json').read_text())
SHAPE = {'entries': [{'group_id': 0, 'field_id': 1, 'type': 2, 'byte_length': 3}], 'consumed': 13}

def report():
    return {'_private_token': VECTORS['empty_groups_token'], 'canvas_verified': True,
        'resources': {'attempted': 0, 'delivered': 0, 'rejected': 0},
        'metadata': {'version': 1, 'sdk': 'framer-embedded-v2', 'selection': 'configured-api',
            'status': 'passed', 'failure_phase': None, 'device_failure': None,
            'configure_calls': 1, 'token_method_calls': 1, 'device_storage_seeded': True,
            'device_storage_unchanged': True, 'window_closed': True, 'sources_unchanged': True,
            'async_errors': 0, 'diagnostics': 0, 'late_settlements': 0, 'method_ms': 10, 'elapsed_ms': 20}}

def raw(value):
    return bytearray(json.dumps(value).encode())

class ProjectionTests(unittest.TestCase):
    def rejected(self, value, describe=lambda token: SHAPE):
        data = raw(value)
        with self.assertRaisesRegex(ValueError, '^token-projection$') as caught:
            PROJECTION.project_report(data, describe)
        self.assertEqual(data, b'')
        self.assertIsNone(caught.exception.__context__)
        self.assertIsNone(caught.exception.__cause__)

    def test_duplicate_keys_including_escaped_names_are_rejected(self):
        text = json.dumps(report())
        duplicates = [
            text.replace('"metadata":', '"metadata": {}, "metadata":', 1),
            text.replace('"device_failure": null',
                         '"device_failure": "local_value", "device_\\u0066ailure": null'),
            text.replace('"attempted": 0', '"attempted": 1, "attempted": 0')]
        for duplicate in duplicates:
            with self.subTest(report=duplicates.index(duplicate)):
                # Each is valid JSON that an ordinary last-value decoder accepts.
                self.assertIsNone(json.loads(duplicate)['metadata']['device_failure'])
                data = bytearray(duplicate.encode())
                with self.assertRaisesRegex(ValueError, '^token-projection$'):
                    PROJECTION.project_report(data, lambda token: SHAPE)
                self.assertEqual(data, b'')

    def test_malformed_oversized_and_wrong_type_inputs_are_rejected(self):
        for value in (b'', b'{', b'\xff', b'null', b'[]', b' ' * 16385):
            data = bytearray(value)
            with self.assertRaisesRegex(ValueError, '^token-projection$'):
                PROJECTION.project_report(data, lambda token: SHAPE)
            self.assertEqual(data, b'')
        for value in ('{}', None, {}, memoryview(b'{}')):
            with self.assertRaisesRegex(ValueError, '^token-projection$'):
                PROJECTION.project_report(value, lambda token: SHAPE)

    def test_failed_status_requires_a_fixed_failure_reason(self):
        value = report()
        value['_private_token'] = None
        value['metadata']['status'] = 'failed'
        self.rejected(value)

    def test_metadata_rejects_wrong_types_nonfinite_and_false_success(self):
        changes = [('version', True), ('configure_calls', True),
            ('diagnostics', -1), ('diagnostics', 1000001), ('method_ms', True),
            ('elapsed_ms', float('nan')), ('method_ms', float('inf')),
            ('elapsed_ms', 16001), ('window_closed', 1), ('sources_unchanged', False),
            ('failure_phase', 'private-value'), ('device_failure', 'cookie_value'),
            ('token_method_calls', 2), ('async_errors', 1), ('late_settlements', 1)]
        for key, bad in changes:
            with self.subTest(field=key, type=type(bad).__name__):
                value = report(); value['metadata'][key] = bad
                self.rejected(value)

    def test_report_resources_and_token_are_strict(self):
        changes = [('extra', 'private-value'), ('canvas_verified', 1),
                   ('_private_token', None), ('_private_token', ''),
                   ('_private_token', 123), ('_private_token', 'x' * 16385)]
        for key, bad in changes:
            value = report(); value[key] = bad
            self.rejected(value)
        for resources in ({'attempted': 0, 'delivered': 1, 'rejected': 0},
                          {'attempted': True, 'delivered': 0, 'rejected': 0},
                          {'attempted': -1, 'delivered': 0, 'rejected': 0},
                          {'attempted': 1, 'delivered': 0, 'rejected': 0, 'url': 'private-value'}):
            value = report(); value['resources'] = resources
            self.rejected(value)

    def test_ephemeral_frame_is_decoded_without_copying_token_to_report(self):
        value = report()
        data = raw(value)
        seen = []
        def describe(token):
            self.assertEqual(FRAME.fingerprint_bytes(token),
                bytes.fromhex(VECTORS['empty_groups_inner']))
            seen.append(True)
            return SHAPE
        result = PROJECTION.project_report(data, describe)
        self.assertEqual(data, b'')
        self.assertEqual(seen, [True])
        self.assertTrue(result['token_discarded'])
        self.assertEqual(result['structure'], SHAPE)
        self.assertNotIn(value['_private_token'], json.dumps(result))
        self.assertNotIn('_private_token', result)
        self.assertIsNot(result['structure']['entries'][0], SHAPE['entries'][0])

    def test_unknown_metadata_and_arbitrary_structure_values_are_rejected(self):
        value = report(); value['metadata']['error'] = 'private-value'
        self.rejected(value)
        shape = copy.deepcopy(SHAPE); shape['entries'][0]['value'] = 'private-value'
        self.rejected(report(), lambda token: shape)
        shape = copy.deepcopy(SHAPE); shape['entries'][0]['field_id'] = 'private-value'
        self.rejected(report(), lambda token: shape)

    def test_structure_string_subclass_keys_are_rejected(self):
        class Text(str):
            pass
        for key in SHAPE['entries'][0]:
            with self.subTest(field=key):
                shape = copy.deepcopy(SHAPE)
                entry = shape['entries'][0]
                entry[Text(key)] = entry.pop(key)
                self.rejected(report(), lambda token: shape)

    def test_disguised_structure_keys_are_rejected(self):
        for key in SHAPE['entries'][0]:
            with self.subTest(field=key):
                class Key(str):
                    def __hash__(self):
                        return hash(key)
                    def __eq__(self, other):
                        return other == key
                shape = copy.deepcopy(SHAPE)
                entry = shape['entries'][0]
                entry[Key('synthetic-key-text')] = entry.pop(key)
                self.assertEqual(set(entry), set(SHAPE['entries'][0]))
                self.assertIn('"synthetic-key-text"', json.dumps(shape))
                self.rejected(report(), lambda token: shape)

    def test_plain_parser_keys_preserve_nonempty_output(self):
        parser = load('v2_token_structure')
        entries, consumed = parser.parse_fingerprint_section(
            bytes.fromhex('00010a008000e0010001200000ff'))
        shape = {'entries': entries, 'consumed': consumed}
        value = report()
        data = raw(value)
        result = PROJECTION.project_report(data, lambda token: shape)
        self.assertEqual(data, b'')
        self.assertEqual(result, {'metadata': value['metadata'], 'token_discarded': True,
            'canvas_verified': True, 'resources': value['resources'], 'structure': {
                'entries': [{'group_id': 0, 'field_id': 1, 'type': 2, 'byte_length': 1}],
                'consumed': 11}})
        self.assertTrue(all(type(key) is str for entry in result['structure']['entries']
                            for key in entry))
        self.assertIsNot(result['structure']['entries'][0], entries[0])

    def test_failed_operation_preserves_fixed_reason_without_decoding(self):
        value = report()
        value['_private_token'] = None
        value['metadata'].update(status='failed', failure_phase='device-storage',
                                 device_failure='local_value', device_storage_unchanged=False)
        def forbidden(token):
            self.fail('failed operation must not decode a token')
        data = raw(value)
        result = PROJECTION.project_report(data, forbidden)
        self.assertEqual(data, b'')
        self.assertIsNone(result['structure'])
        self.assertFalse(result['token_discarded'])
        self.assertEqual(result['metadata']['device_failure'], 'local_value')
        value['_private_token'] = 'private-value'
        self.rejected(value, forbidden)

    def test_decoder_errors_have_fixed_message_and_clear_input(self):
        def broken(token):
            raise RuntimeError('private-value')
        self.rejected(report(), broken)
        data = raw(report())
        try:
            PROJECTION.project_report(data, broken)
        except ValueError as error:
            self.assertEqual(error.args, ('token-projection',))
            self.assertTrue(error.__suppress_context__)
            self.assertIsNone(error.__context__)
            self.assertIsNone(error.__cause__)
        else:
            self.fail('decoder failure was accepted')
        self.assertEqual(data, b'')

    def test_control_flow_failure_propagates_and_still_clears_input(self):
        class Cancelled(BaseException):
            pass
        failure = Cancelled()
        def interrupted(token):
            raise failure
        data = raw(report())
        with self.assertRaises(Cancelled) as caught:
            PROJECTION.project_report(data, interrupted)
        self.assertIs(caught.exception, failure)
        self.assertEqual(data, b'')

    def test_structure_requires_bounded_distinct_ordered_fields_and_exact_size(self):
        shapes = [None, {'entries': [], 'consumed': 11},
            {'entries': [], 'consumed': True},
            {'entries': [], 'consumed': 16385},
            {'entries': SHAPE['entries'] * 2, 'consumed': 16}]
        for key, bad in [('group_id', 1), ('group_id', True), ('field_id', -1),
                         ('field_id', 32), ('type', True), ('type', 8),
                         ('byte_length', 0), ('byte_length', 16385)]:
            shape = copy.deepcopy(SHAPE); shape['entries'][0][key] = bad
            shapes.append(shape)
        reverse = copy.deepcopy(SHAPE)
        reverse['entries'].insert(0, dict(group_id=9, field_id=0, type=0, byte_length=1))
        reverse['consumed'] += 1
        shapes.append(reverse)
        for shape in shapes:
            self.rejected(report(), lambda token: shape)

    def test_empty_groups_and_immutable_pipe_bytes_are_supported(self):
        result = PROJECTION.project_report(bytes(raw(report())),
                                          lambda token: {'entries': [], 'consumed': 10})
        self.assertEqual(result['structure'], {'entries': [], 'consumed': 10})
        self.assertEqual(set(result), {'metadata', 'token_discarded', 'canvas_verified',
                                       'resources', 'structure'})


if __name__ == '__main__':
    unittest.main()
