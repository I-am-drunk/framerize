"""Pure validation and restoration of rate limits, excluding credentials."""
import math, re
from collections import deque

FIELDS = {'cooling', 'requests', 'last_mint', 'reserve_day', 'reserve_used',
          'primary_device', 'ambient_cooling', 'ambient_requests'}

def require(ok, reason):
    if not ok:
        raise ValueError(reason)

def number(value):
    require(type(value) in (int, float) and math.isfinite(value) and value >= 0,
            'invalid monotonic timestamp')
    return value

def validate(payload, boot_id, now, devices):
    require(set(payload) == {'format', 'boot_id', 'captured_monotonic', 'state'},
            'unexpected snapshot fields')
    require(type(payload['format']) is int and payload['format'] == 1 and payload['boot_id'] == boot_id,
            'snapshot format or boot differs')
    captured = number(payload['captured_monotonic'])
    require(captured <= now, 'snapshot is from the future')
    state = payload['state']
    require(set(state) == {'generation', 'accounts'}, 'unexpected host fields')
    require(re.fullmatch('[0-9a-f]{32}', state['generation']) is not None,
            'invalid source generation')
    rows = state['accounts']
    require(type(rows) is dict and len(rows) <= 64 and set(rows) <= set(devices),
            'snapshot account inventory differs')
    for account, row in rows.items():
        validate_account(row, captured, set(devices[account]))
    return rows

def validate_account(row, captured, devices):
    require(type(row) is dict and set(row) == FIELDS, 'unexpected account fields')
    require(type(row['cooling']) is dict and type(row['requests']) is dict,
            'invalid per-device ledger')
    require(set(row['cooling']) | set(row['requests']) <= devices,
            'snapshot device inventory differs')
    require(row['primary_device'] is None or row['primary_device'] in devices,
            'unknown primary device')
    require(type(row['reserve_used']) is int and 0 <= row['reserve_used'] <= 1000000,
            'invalid reserve count')
    require(type(row['reserve_day']) is str and
            re.fullmatch(r'|\d{4}-\d{2}-\d{2}', row['reserve_day']) is not None,
            'invalid reserve day')
    require(number(row['last_mint']) <= captured, 'future mint timestamp')
    for until in row['cooling'].values():
        require(number(until) <= captured + 600, 'unbounded device cooldown')
    if row['ambient_cooling'] is not None:
        require(number(row['ambient_cooling']) <= captured + 600, 'unbounded ambient cooldown')
    windows = list(row['requests'].values())
    if row['ambient_requests'] is not None:
        windows.append(row['ambient_requests'])
    for times in windows:
        require(type(times) is list and len(times) <= 2048, 'unbounded request window')
        require(all(number(stamp) <= captured for stamp in times), 'future request')
        require(times == sorted(times), 'unordered request window')

def restore(host, lifecycle_type, rows):
    require(not host.lifecycles and not host.accounts, 'host is not fresh')
    for account, row in rows.items():
        life = lifecycle_type()
        life.cooling = dict(row['cooling'])
        life.requests = {device: deque(times) for device, times in row['requests'].items()}
        if row['ambient_cooling'] is not None:
            life.cooling[None] = row['ambient_cooling']
        if row['ambient_requests'] is not None:
            life.requests[None] = deque(row['ambient_requests'])
        life.last_mint, life.reserve_day = row['last_mint'], row['reserve_day']
        life.reserve_used = row['reserve_used']
        life.restored_primary_device = row['primary_device']
        host.lifecycles[account] = life
