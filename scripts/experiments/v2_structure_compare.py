"""Compare projected V2 field structure, without tokens, payloads or I/O."""

from v2_token_projection import _structure


def _index(shape):
    return {(row['group_id'], row['field_id']):
            {'type': row['type'], 'byte_length': row['byte_length']}
            for row in shape['entries']}


def compare_structures(baseline, native):
    """Compare two projection shapes; None means an unavailable observation.

    The caller owns provenance, pairing, minting and all lifecycle checks.
    A structural match says nothing about payload equality or acceptance.
    Failures expose only a fixed error message, without input details.
    """
    try:
        left = None if baseline is None else _structure(baseline)
        right = None if native is None else _structure(native)
        result = {
            'comparable': left is not None and right is not None,
            'structural_match': None,
            'baseline_consumed': None if left is None else left['consumed'],
            'native_consumed': None if right is None else right['consumed'],
            'differences': [],
        }
        if not result['comparable']:
            return result
        before, after = _index(left), _index(right)
        for group, field in sorted(before.keys() | after.keys()):
            a, b = before.get((group, field)), after.get((group, field))
            if a != b:
                result['differences'].append({
                    'group_id': group, 'field_id': field,
                    'baseline': a, 'native': b,
                })
        result['structural_match'] = not result['differences']
        return result
    except Exception:
        pass
    # Leave the handler before raising so the caught error is not its context.
    raise ValueError('structure-comparison') from None
