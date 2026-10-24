"""Check a packaged host's real entrypoint using empty isolated state."""
import argparse
import json
import os
from pathlib import Path
import selectors
import subprocess
import time
import uuid


def exchange(child, request):
    child.stdin.write(json.dumps(request).encode() + b'\n')
    child.stdin.flush()
    data = bytearray()
    deadline = time.monotonic() + 5
    with selectors.DefaultSelector() as selector:
        selector.register(child.stdout, selectors.EVENT_READ)
        while b'\n' not in data:
            remaining = deadline - time.monotonic()
            if remaining <= 0 or not selector.select(remaining):
                raise RuntimeError('host reply deadline')
            part = os.read(child.stdout.fileno(), 8193 - len(data))
            if not part or len(data) + len(part) > 8192:
                raise RuntimeError('missing or oversized host reply')
            data.extend(part)
    reply = json.loads(data)
    if reply.get('id') != request['id'] or reply.get('ok') is not True:
        raise RuntimeError('unexpected host reply')
    return reply['result']


def children(pid):
    return sorted({child for task in Path('/proc', str(pid), 'task').iterdir()
                   for child in (task / 'children').read_text().split()})


def check(package, output):
    for kind in ('net', 'pid'):
        if os.readlink('/proc/self/ns/' + kind) == os.environ['CHECK_PARENT_' + kind.upper() + 'NS']:
            raise RuntimeError('private namespace required')
    if Path('/home/tjninja/.framerize').exists() or Path('/run/user/1000/bus').exists():
        raise RuntimeError('host runtime visible')
    output.mkdir(mode=0o700)
    home, runtime = output / 'home', output / 'runtime'
    home.mkdir(mode=0o700)
    state = runtime / 'profiles/.sidecar/camoufox'
    argv = ['/usr/bin/python3.12', '-s', '-S', '-B', '-X',
            'pycache_prefix=' + str(home / 'unused-pycache'),
            str(package / 'host/camoufox_host.py')]
    env = os.environ | {'HOME': str(home), 'FRAMERIZE_STATE': str(state),
                        'FRAMERIZE_SIDECAR_MODE': 'unmanaged'}
    report = {'accepted': False, 'argv': argv, 'checks': {}}
    account = '00000000-0000-4000-8000-000000000001'
    with (output / 'host.log').open('xb') as log:
        child = subprocess.Popen(argv, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                 stderr=log, env=env)
        report['host_pid_in_namespace'] = child.pid
        try:
            status = exchange(child, {'id': 1, 'cmd': 'browser_status', 'account': account})
            expected = dict(account_id=account, account_incarnation=0,
                            state='absent', active_commands=0)
            generation = status.get('host_generation')
            if type(generation) is not str or uuid.UUID(generation).hex != generation:
                raise RuntimeError('missing host generation')
            if status != expected | {'host_generation': generation}:
                raise RuntimeError('unexpected empty host state')
            report['checks']['empty_status'] = True
            refusal = exchange(child, dict(id=2, cmd='native_device_open', account=account,
                lease='00000000-0000-4000-8000-000000000002'))
            if refusal != {'rejected': True}:
                raise RuntimeError('missing native ownership was not refused')
            report['checks']['native_ownership_refusal'] = True
            if exchange(child, dict(id=3, cmd='browser_status', account=account)) != status:
                raise RuntimeError('refusal changed empty host state')
            report['checks']['same_generation_empty_status'] = True
            report['children_before_eof'] = children(child.pid)
            if report['children_before_eof']:
                raise RuntimeError('unexpected child process')
            child.stdin.close()
            if child.wait(timeout=5) != 0 or child.stdout.read(8193):
                raise RuntimeError('unclean EOF shutdown or extra output')
            report['checks']['eof_shutdown'] = True
            if any(p.is_file() or p.is_symlink() for p in home.rglob('*')) or any(
                    p.is_file() or p.is_symlink() for p in runtime.rglob('*')):
                raise RuntimeError('unexpected persistent state')
            report['checks']['no_persistent_files'] = True
            report['accepted'] = True
        finally:
            if child.poll() is None:
                child.kill()
                child.wait(timeout=3)
                report['forced_kill'] = True
                report['accepted'] = False
            child.stdout.close()
            if not child.stdin.closed:
                child.stdin.close()
            report['exit_code'] = child.returncode
            report['host_absent'] = not Path('/proc', str(child.pid)).exists()
            report['accepted'] &= report['host_absent']
            (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')
    if not report['accepted']:
        raise RuntimeError('host startup check failed')
    print(json.dumps(report))


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('package', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    check(args.package, args.output)
