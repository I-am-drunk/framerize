"""Synchronous, fixed-fact observation of an already returned resident token."""

import uuid

from v2_token_projection import _structure


def record(token, lease, phase, describe, emit):
    """Describe once and emit only validated structure; return no token or value.

    Both ports must be synchronous. Invalid context invokes neither port.
    Ordinary failures are contained; process/control-flow exceptions propagate.
    The caller owns minting, decoding, event delivery and all lifecycle policy.
    """
    try:
        if (type(lease) is not str or len(lease) != 36
                or type(phase) is not str or phase not in ('baseline', 'native')):
            return
        identity = uuid.UUID(lease)
        if not identity.int or str(identity) != lease:
            return
    except Exception:
        return

    structure = None
    try:
        described = describe(token)
        validated = _structure(described)
        # Equal keys can still be string subclasses carrying different text.
        if (any(type(key) is not str for key in described)
                or any(type(key) is not str for entry in validated['entries']
                       for key in entry)):
            raise ValueError()
        structure = validated
    except Exception:
        pass

    try:
        emit({'event': 'resident.mint.structure', 'lease': lease, 'phase': phase,
              'available': structure is not None, 'structure': structure})
    except Exception:
        pass
