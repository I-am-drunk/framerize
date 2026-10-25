"""Restore captured limits before the new host accepts its first command."""
import ctypes
import hashlib
import os
import re
import stat
import time
from pathlib import Path
from . import inventory, ledger, private_state


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def runtime_root(device_root):
    """Resolve state from this process's profiles, never its shared code location."""
    path = Path(device_root)
    ledger.require(path.is_absolute() and path.resolve() == path,
                   'noncanonical runtime state path')
    ledger.require(path.name == 'camoufox' and path.parent.name == '.sidecar'
                   and path.parent.parent.name == 'profiles',
                   'runtime state must use root/profiles/.sidecar/camoufox')
    return path.parent.parent.parent


def process(pid):
    root = Path('/proc', str(pid))
    def ticks():
        return int((root / 'stat').read_text().rsplit(') ', 1)[1].split()[19])
    start = ticks()
    result = {'pid': pid, 'start': start, 'exe_sha256': digest(root / 'exe')}
    ledger.require(ticks() == start, 'process changed while reading identity')
    return result


def is_absent(prior):
    try:
        stat_text = Path('/proc', str(prior['pid']), 'stat').read_text()
    except FileNotFoundError:
        return True
    return int(stat_text.rsplit(') ', 1)[1].split()[19]) != prior['start']


def checked_request(request, directory, script, boot, package_hash):
    require = ledger.require
    require(set(request) == {'format', 'nonce', 'boot_id', 'source',
                            'destination', 'snapshot', 'inventory'},
            'unexpected handoff request fields')
    require(type(request['format']) is int and request['format'] == 1,
            'unsupported handoff format')
    nonce = request['nonce']
    require(type(nonce) is str and re.fullmatch('[0-9a-f]{32}', nonce),
            'invalid handoff nonce')
    require(request['boot_id'] == boot, 'handoff belongs to a different boot')
    path = directory / ('snapshot-' + nonce + '.json')
    require(request['snapshot']['path'] == str(path), 'snapshot path differs')
    source, target = request['source'], request['destination']
    require(package_hash is not None and target.get('runtime_sha256') == package_hash,
            'handoff does not bind this runtime package')
    require(all(is_absent(source[name]) for name in ('main', 'sidecar')),
            'source generation still exists')
    parent = process(os.getppid())
    require(parent['exe_sha256'] == target['binary_sha256'], 'parent binary differs')
    require(digest(script) == target['sidecar_sha256'], 'startup script differs')
    invocation = os.environ.get('INVOCATION_ID', '')
    require(re.fullmatch('[0-9a-f]{32}', invocation) and
            invocation != source['invocation_id'], 'service invocation differs')
    group = Path('/proc/self/cgroup').read_text().strip()
    require(group == '0::' + source['cgroup'], 'service boundary differs')
    return path, nonce, parent, invocation


def permit_controller(expected):
    ledger.require(process(expected['pid']) == expected, 'controller identity differs')
    libc = ctypes.CDLL(None, use_errno=True)
    libc.prctl.argtypes = [ctypes.c_int] + [ctypes.c_ulong] * 4
    libc.prctl.restype = ctypes.c_int
    result = libc.prctl(0x59616d61, expected['pid'], 0, 0, 0)  # PR_SET_PTRACER
    ledger.require(result == 0, 'cannot bind the capture controller')
    ledger.require(process(expected['pid']) == expected, 'controller changed')


def restore_host(host, lifecycle_type, root, script, package_hash=None):
    directory = Path(root) / 'cooldown-handoff'
    pending = directory / 'pending.json'
    if not pending.exists() and not pending.is_symlink():
        return None
    mode = directory.lstat()
    ledger.require(stat.S_ISDIR(mode.st_mode) and mode.st_uid == os.getuid()
                   and stat.S_IMODE(mode.st_mode) == 0o700, 'unsafe handoff directory')
    request, request_hash = private_state.read(pending)
    boot = Path('/proc/sys/kernel/random/boot_id').read_text().strip()
    path, nonce, parent, invocation = checked_request(request, directory, script, boot, package_hash)
    payload, payload_hash = private_state.read(path, request['snapshot']['sha256'])
    current, devices = inventory.collect(root)
    ledger.require(current == request['inventory'], 'saved account state changed')
    rows = ledger.validate(payload, boot, time.monotonic(), devices)
    ledger.require(payload['state']['generation'] == request['source']['host_generation'],
                   'snapshot belongs to another host generation')
    return commit_restore(host, lifecycle_type, rows, request, request_hash,
                          payload_hash, directory, parent, invocation)


def commit_restore(host, lifecycle_type, rows, request, request_hash,
                   payload_hash, directory, parent, invocation):
    ledger.require(not host.accounts and not host.lifecycles, 'host is not fresh')
    nonce = request['nonce']
    owner = process(os.getpid())
    claim = {'nonce': nonce, 'request_sha256': request_hash, 'sidecar': owner,
             'main': parent, 'invocation_id': invocation}
    private_state.write_new(directory / ('claim-' + nonce + '.json'), claim)
    permit_controller(request['destination']['controller'])
    ledger.restore(host, lifecycle_type, rows)
    receipt = dict(claim, status='restored', snapshot_sha256=payload_hash,
                   source_generation=request['source']['host_generation'],
                   generation=host.generation, account_count=len(rows),
                   restored_monotonic=time.monotonic())
    private_state.write_new(directory / ('restored-' + nonce + '.json'), receipt)
    host._cooldown_restoration = receipt
    return receipt


def restore_primary(acc):
    """Restore sticky device selection after account initialization."""
    if hasattr(acc.lifecycle, 'restored_primary_device'):
        primary = acc.lifecycle.restored_primary_device
        ledger.require(primary is None or primary in acc.devices,
                       'restored primary device disappeared')
        acc.primary_dev = primary
        del acc.lifecycle.restored_primary_device
