"""Explicit pre-admission expiry. The parent keeps session ownership."""
import json
import time
from pathlib import Path

from framerize_accounts.expiry import ExpiryWitness


def expire_legacy(root, previous_runtime, *, sleep=time.sleep, clock=None):
    with Path(previous_runtime).open('rb') as stream:
        raw = stream.read(4097)
    if len(raw) > 4096:
        raise ValueError('previous runtime metadata exceeds limit')
    source = json.loads(raw)
    with ExpiryWitness(root, source, clock=clock) as witness:
        while remaining := witness.remaining():
            sleep(min(remaining, 1.0))
        return witness.install()
