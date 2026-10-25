"""Rebind the qualified read-only descriptor protocol to this controller."""
import importlib.util, json, os, stat, sys
from pathlib import Path
from runtime import HERE, need, sha
SOURCE = HERE.parents[1]/'offline-serving-r18-runtime-recovery-r3/fd_bridge.py'
SOURCE_SHA = '121bf968b998358b3f9ecc530ff8ab77b2ca5788b034324a33efd51bab3749c2'

def bridge(plan, purpose):
    need(sha(SOURCE) == SOURCE_SHA, 'descriptor_source_changed')
    spec = importlib.util.spec_from_file_location('bound_readonly_descriptor', SOURCE)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    module.NEW_BINARY = plan['binary_sha256']
    module.NEW_SCRIPT = plan['sidecar_sha256']
    module.OLD_BINARY = plan['rollback']['binary_sha256']
    module.OLD_SCRIPT = plan['rollback']['sidecar_sha256']
    module.SIDECAR = Path(plan['sidecar'] if purpose == 'rollback'
                          else plan['rollback']['sidecar'])
    module.read_request = read_request
    return module

def read_request(expected, uid, purpose='activate'):
    path = HERE/'output'/purpose/'memory-access.json'
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        st = os.fstat(fd)
        need(stat.S_ISREG(st.st_mode) and st.st_uid == uid and
             stat.S_IMODE(st.st_mode) == 0o600 and st.st_nlink == 1,
             'unsafe_descriptor_request')
        import hashlib
        raw = os.read(fd, 16385)
        need(len(raw) <= 16384 and hashlib.sha256(raw).hexdigest() == expected,
             'descriptor_request_changed')
        return json.loads(raw)
    finally:
        os.close(fd)

def receive(plan, before, purpose, save):
    module = bridge(plan, purpose)
    return module.receive_descriptor(before, HERE/'output'/purpose, save)
