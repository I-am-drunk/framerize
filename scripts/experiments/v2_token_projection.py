"""Project an ephemeral SDK report without retaining its private token."""
import json
import math

_FLAGS = {'device_storage_seeded', 'device_storage_unchanged',
          'window_closed', 'sources_unchanged'}
_COUNTS = {'configure_calls', 'token_method_calls', 'async_errors',
           'diagnostics', 'late_settlements'}
_TIMES = {'method_ms', 'elapsed_ms'}
_FIXED = {'version': 1, 'sdk': 'framer-embedded-v2', 'selection': 'configured-api'}
_PHASES = {None, 'input-identity', 'runtime', 'window', 'device-storage',
           'configure', 'mint', 'cleanup', 'deadline', 'token-shape'}
_DEVICE = {None, 'local_read', 'local_value', 'cookie_read', 'cookie_count', 'cookie_value'}


def _pairs(items):
    result = {}
    for key, value in items:
        if key in result:
            raise ValueError()
        result[key] = value
    return result


def _integer(value, low, high):
    return type(value) is int and low <= value <= high


def _metadata(meta):
    if type(meta) is not dict or set(meta) != (
            set(_FIXED) | _FLAGS | _COUNTS | _TIMES | {'status', 'failure_phase', 'device_failure'}):
        raise ValueError()
    if any(type(meta[key]) is not type(value) or meta[key] != value
           for key, value in _FIXED.items()):
        raise ValueError()
    if any(type(meta[key]) is not bool for key in _FLAGS):
        raise ValueError()
    if any(not _integer(meta[key], 0, 1000000) for key in _COUNTS):
        raise ValueError()
    if any(type(meta[key]) not in (int, float) or not math.isfinite(meta[key])
           or not 0 <= meta[key] <= 16000 for key in _TIMES):
        raise ValueError()
    if meta['status'] not in ('passed', 'failed') or meta['failure_phase'] not in _PHASES:
        raise ValueError()
    if meta['device_failure'] not in _DEVICE:
        raise ValueError()
    if meta['status'] == 'failed' and meta['failure_phase'] is None:
        raise ValueError()
    if meta['status'] == 'passed' and (not all(meta[key] for key in _FLAGS)
            or meta['failure_phase'] is not None or meta['device_failure'] is not None
            or meta['configure_calls'] != 1 or meta['token_method_calls'] != 1
            or any(meta[key] for key in ('async_errors', 'late_settlements'))):
        raise ValueError()


def _structure(shape):
    if type(shape) is not dict or set(shape) != {'entries', 'consumed'}:
        raise ValueError()
    if not _integer(shape['consumed'], 10, 16384):
        raise ValueError()
    if type(shape['entries']) is not list or len(shape['entries']) > 155:
        raise ValueError()
    seen = set()
    previous_group = -1
    total = 10  # The five two-byte group headers are not field entries.
    for entry in shape['entries']:
        if type(entry) is not dict or set(entry) != {'group_id', 'field_id', 'type', 'byte_length'}:
            raise ValueError()
        if type(entry['group_id']) is not int or entry['group_id'] not in (0, 4, 7, 8, 9):
            raise ValueError()
        if not _integer(entry['field_id'], 0, 31) or not _integer(entry['type'], 0, 7):
            raise ValueError()
        if not _integer(entry['byte_length'], 1, 16384):
            raise ValueError()
        key = (entry['group_id'], entry['field_id'])
        if key in seen or entry['group_id'] < previous_group:
            raise ValueError()
        seen.add(key)
        previous_group = entry['group_id']
        total += entry['byte_length']
    if total != shape['consumed']:
        raise ValueError()
    return {'entries': [dict(entry) for entry in shape['entries']], 'consumed': shape['consumed']}


def project_report(raw, describe):
    """Consume bounded pipe bytes; return fixed metadata and structural facts.

    ``describe`` returns only entries and consumed bytes from the parser.
    Mutable input is cleared on every path. This releases references; Python
    does not guarantee secure erasure of the decoded immutable token string.
    """
    try:
        if type(raw) not in (bytes, bytearray) or not 0 < len(raw) <= 16384:
            raise ValueError()
        report = json.loads(raw, object_pairs_hook=_pairs)
        if type(report) is not dict or set(report) != {
                'metadata', '_private_token', 'canvas_verified', 'resources'}:
            raise ValueError()
        meta = report['metadata']
        _metadata(meta)
        if report['canvas_verified'] is not True:
            raise ValueError()
        resources = report['resources']
        if type(resources) is not dict or set(resources) != {'attempted', 'delivered', 'rejected'}:
            raise ValueError()
        if not all(_integer(value, 0, 1000000) for value in resources.values()):
            raise ValueError()
        if resources['delivered'] + resources['rejected'] > resources['attempted']:
            raise ValueError()
        token = report.pop('_private_token')
        if meta['status'] == 'passed':
            if type(token) is not str or not 0 < len(token) <= 16384:
                raise ValueError()
            shape = _structure(describe(token))
            # Equal keys may be string subclasses carrying different text.
            if any(type(key) is not str for entry in shape['entries'] for key in entry):
                raise ValueError()
        else:
            if token is not None:
                raise ValueError()
            shape = None
        return {'metadata': dict(meta), 'token_discarded': token is not None,
                'canvas_verified': True, 'resources': dict(resources), 'structure': shape}
    except Exception:
        pass
    finally:
        if type(raw) is bytearray:
            raw.clear()
    # Detach the caught failure after leaving its handler and clearing input.
    raise ValueError('token-projection') from None
