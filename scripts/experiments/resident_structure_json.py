"""Decode a bounded event line before separate envelope and pair validation."""
import json

from v2_token_projection import _pairs


def decode_record(raw):
    """Reject duplicate keys at every depth; return no partial record on error.

    This is JSON framing only, not schema validation or capture provenance.
    The bound matches the collector's existing 32 MiB file bound.
    """
    try:
        if type(raw) is not bytes or not 0 < len(raw) < 32 * 1024**2:
            raise ValueError()
        value = json.loads(raw, object_pairs_hook=_pairs)
        if type(value) is dict:
            return value
    except Exception:
        pass
    raise ValueError('resident-structure-json') from None
