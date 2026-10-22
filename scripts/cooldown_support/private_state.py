"""Bounded private snapshot files; no credentials are serialized here."""
import hashlib, json, os, stat
from pathlib import Path

LIMIT = 256 * 1024

def unique(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('duplicate snapshot key')
        result[key] = value
    return result

def read(path, expected=None):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK)
    try:
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid():
            raise ValueError('snapshot ownership or type differs')
        if stat.S_IMODE(info.st_mode) != 0o600 or info.st_nlink != 1:
            raise ValueError('snapshot must be private and singly linked')
        if not 0 <= info.st_size <= LIMIT:
            raise ValueError('snapshot exceeds size bound')
        raw = os.read(fd, LIMIT + 1)
        if len(raw) > LIMIT:
            raise ValueError('snapshot exceeds size bound')
        digest = hashlib.sha256(raw).hexdigest()
        if expected is not None and digest != expected:
            raise ValueError('snapshot hash differs')
        after = os.fstat(fd)
        fields = ('st_size', 'st_mtime_ns', 'st_ctime_ns')
        if len(raw) != info.st_size or any(getattr(info, f) != getattr(after, f) for f in fields):
            raise ValueError('snapshot changed while reading')
        return json.loads(raw, object_pairs_hook=unique), digest
    finally:
        os.close(fd)

def write_new(path, value):
    path = Path(path)
    raw = (json.dumps(value, sort_keys=True, allow_nan=False) + '\n').encode()
    if len(raw) > LIMIT:
        raise ValueError('snapshot exceeds size bound')
    temporary = path.with_name('.' + path.name + '.tmp')
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    published = False
    try:
        with os.fdopen(fd, 'wb') as out:
            out.write(raw)
            out.flush()
            os.fsync(out.fileno())
        os.link(temporary, path, follow_symlinks=False)
        published = True
        temporary.unlink()
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        if temporary.exists():
            temporary.unlink()
    if not published:
        raise RuntimeError('snapshot was not published')
    return hashlib.sha256(raw).hexdigest()
