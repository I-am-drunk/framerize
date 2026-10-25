"""Verify the runtime files shipped together in an application archive."""
import hashlib
import json
from pathlib import Path


def verify_runtime(root):
    root = Path(root).resolve()
    manifest = root / 'runtime-manifest.json'
    if not manifest.exists():
        return None  # Source checkout, selected explicitly for development.
    raw = manifest.read_bytes()
    entries = json.loads(raw)
    if type(entries) is not dict or not entries:
        raise ValueError('invalid runtime manifest')
    actual = {p.relative_to(root).as_posix() for p in root.rglob('*')
              if p.is_file() and p != manifest and '__pycache__' not in p.parts}
    if actual != set(entries):
        raise ValueError('runtime package file inventory changed')
    for name, expected in entries.items():
        path = root / name
        if path.resolve() != path or not path.is_file():
            raise ValueError('invalid runtime package path')
        with path.open('rb') as stream:
            if hashlib.file_digest(stream, 'sha256').hexdigest() != expected:
                raise ValueError('runtime package file changed: ' + name)
    return hashlib.sha256(raw).hexdigest()
