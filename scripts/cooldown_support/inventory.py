"""Logical account and stable-device identities; no credential output."""
import hashlib, json, sqlite3, uuid
from pathlib import Path

def collect(root):
    root = Path(root)
    records, identities, devices, device_hashes = {}, {}, {}, {}
    with sqlite3.connect('file:' + str(root / 'framerize.db') + '?mode=ro', uri=True) as db:
        db.execute('BEGIN')
        count, maximum = db.execute('SELECT count(*), max(length(json)) FROM accounts').fetchone()
        if count > 64 or (maximum or 0) > 65536:
            raise ValueError('account inventory exceeds bound')
        rows = db.execute('SELECT json FROM accounts').fetchall()
    if len(rows) > 64:
        raise ValueError('account inventory exceeds bound')
    for (raw,) in rows:
        row = json.loads(raw)
        aid = str(uuid.UUID(row['id']))
        if aid != row['id'] or aid in devices:
            raise ValueError('duplicate saved account')
        key = hashlib.sha256(aid.encode()).hexdigest()
        records[key] = hashlib.sha256(json.dumps(row, sort_keys=True).encode()).hexdigest()
        identity = {k: row.get(k) for k in ('id', 'alias', 'email', 'framer_user_id',
                                          'castle_device_id', 'proxy')}
        identities[key] = hashlib.sha256(json.dumps(identity, sort_keys=True).encode()).hexdigest()
        path = root / 'profiles/.sidecar/camoufox' / aid / 'framerize-devices.json'
        raw_devices = None
        if path.exists():
            with path.open('rb') as stream:
                raw_devices = stream.read(16385)
            if len(raw_devices) > 16384:
                raise ValueError('device inventory exceeds bound')
        devices[aid] = json.loads(raw_devices) if raw_devices is not None else []
        if type(devices[aid]) is not list or len(devices[aid]) > 64:
            raise ValueError('invalid saved device inventory')
        if any(type(d) is not str or not 1 <= len(d) <= 256 for d in devices[aid]):
            raise ValueError('invalid saved device identity')
        if len(set(devices[aid])) != len(devices[aid]):
            raise ValueError('duplicate device identity')
        device_hashes[key] = hashlib.sha256(raw_devices).hexdigest() if raw_devices is not None else None
    return {'records': records, 'identities': identities, 'device_hashes': device_hashes}, devices
