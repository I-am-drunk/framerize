import hashlib
import json
from pathlib import Path
import stat


class Refusal(Exception):
    """Only locally selected codes may appear in a public failure report."""


def require(ok, code):
    if not ok:
        raise Refusal(code)


def error_code(error):
    return str(error) if type(error) is Refusal else type(error).__name__


def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def read_json(path, maximum=65536, private=False, digest=None):
    path = Path(path)
    require(path.is_absolute() and path.resolve(strict=True) == path, 'noncanonical_input')
    mode = path.stat().st_mode
    require(stat.S_ISREG(mode), 'input_not_regular')
    if private:
        require(mode & 0o077 == 0, 'input_permissions')
    with path.open('rb') as stream:
        raw = stream.read(maximum + 1)
    require(len(raw) <= maximum, 'input_byte_limit')
    if digest is not None:
        require(hashlib.sha256(raw).hexdigest() == digest, 'input_changed')
    return json.loads(raw)
