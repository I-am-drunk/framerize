"""Payload-free matching of original producer joins and both releases."""
import datetime
import uuid
from support import require

KINDS = {'operation.registered', 'operation.joined', 'operation.released',
         'operation.unresolved', 'operation.admission.refused',
         'ai.post.started', 'ai.post', 'ai.chain', 'ai.chain.repick'}
FIELDS = {'ts', 'event', 'request_id', 'operation_id', 'parent_operation_id',
          'kind', 'outcome', 'cleanup_confirmed', 'status', 'chunks',
          'metered_chunks'}

def root_id(value):
    try:
        parsed = uuid.UUID(value)
    except (ValueError, TypeError, AttributeError):
        raise RuntimeError('invalid_operation_header') from None
    require(parsed.int != 0 and str(parsed) == value, 'invalid_operation_header')
    return value

class Evidence:
    def __init__(self):
        self.rows = []

    def add(self, row):
        if row.get('event') in KINDS:
            require(len(self.rows) < 2048, 'event_count_limit')
            self.rows.append({k: v for k, v in row.items() if k in FIELDS})

    def count(self, event):
        return sum(r['event'] == event for r in self.rows)

    def joined(self, root, after=None):
        root_id(root)
        rows = [r for r in self.rows if r.get('request_id') == root]
        require(not any(r['event'] == 'operation.unresolved' for r in rows),
                'unresolved_operation')
        registered = [r for r in rows if r['event'] == 'operation.registered']
        roots = [r for r in registered if r.get('operation_id') == root and
                 r.get('parent_operation_id') is None]
        children = [r['operation_id'] for r in registered if
                    r.get('parent_operation_id') == root]
        require(len(roots) <= 1 and len(children) <= 1, 'ambiguous_operation')
        if len(roots) != 1 or len(children) != 1:
            return False
        child = root_id(children[0])
        expected = [('operation.joined', child), ('operation.released', child),
                    ('operation.released', root)]
        positions = []
        for event, operation in expected:
            found = [(i, r) for i, r in enumerate(rows) if r['event'] == event
                     and r.get('operation_id') == operation]
            require(len(found) <= 1, 'duplicate_completion_event')
            if not found:
                return False
            i, row = found[0]
            require(row.get('cleanup_confirmed') is True, 'cleanup_unconfirmed')
            if after is not None:
                stamp = datetime.datetime.fromisoformat(row['ts'].replace('Z', '+00:00'))
                require(stamp.timestamp() > after, 'completion_predates_disconnect')
            positions.append(i)
        require(positions == sorted(positions), 'completion_order_wrong')
        return True
