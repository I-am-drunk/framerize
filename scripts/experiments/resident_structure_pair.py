"""Pair already-selected resident structural observations without token I/O."""

import uuid

from v2_token_projection import _structure


def _context(record, phase):
    if (type(record) is not dict or len(record) != 5
            or any(type(key) is not str for key in record)
            or set(record) != {'event', 'lease', 'phase', 'available', 'structure'}):
        raise ValueError()
    lease = record['lease']
    if (type(record['event']) is not str or record['event'] != 'resident.mint.structure'
            or type(record['phase']) is not str or record['phase'] != phase
            or type(lease) is not str or len(lease) != 36
            or type(record['available']) is not bool):
        raise ValueError()
    identity = uuid.UUID(lease)
    if not identity.int or str(identity) != lease:
        raise ValueError()
    return lease


def _shape(record):
    shape = record['structure']
    if not record['available']:
        if shape is not None:
            raise ValueError()
        return None
    validated = _structure(shape)
    # Equal-looking subclass keys can serialize as different, private text.
    if (any(type(key) is not str for key in shape)
            or any(type(key) is not str for entry in validated['entries'] for key in entry)):
        raise ValueError()
    return {'entries': [{'group_id': entry['group_id'], 'field_id': entry['field_id'],
                         'type': entry['type'], 'byte_length': entry['byte_length']}
                        for entry in validated['entries']], 'consumed': validated['consumed']}


def pair_records(baseline, native):
    """Join one selected event per phase; the caller owns unique event selection."""
    try:
        before = _context(baseline, 'baseline')
        after = _context(native, 'native')
        if before != after:
            raise ValueError()
        return {'same_lease': True,
                'structures': {'baseline': _shape(baseline), 'native': _shape(native)}}
    except Exception:
        pass
    # Raise after leaving the handler so its exception is not retained as context.
    raise ValueError('resident-structure-pair') from None
