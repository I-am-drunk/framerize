"""Measure one synthetic native host lease with the real owned SDK worker."""
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import uuid

from restored_native_host import Lab, sha


def group_root():
    return Path('/sys/fs/cgroup' + Path('/proc/self/cgroup').read_text().strip().split('::', 1)[1])


def sample(group):
    return {name: int((group / name).read_text()) for name in (
        'memory.current', 'memory.peak', 'memory.max', 'memory.swap.current',
        'memory.swap.peak', 'memory.swap.max', 'pids.current', 'pids.peak', 'pids.max')}


def check_status(lab, child, account, generation, active):
    status = lab.command(child, 'browser_status', account=account)
    assert status['host_generation'] == generation, 'host generation changed'
    assert (status['state'], status['active_commands']) == ('absent', active)
    return status


def worker_command(source, output):
    settings = json.loads((source / 'mounts.json').read_text())
    argv = ['/usr/bin/bwrap', '--die-with-parent', '--new-session',
        '--unshare-net', '--unshare-pid', '--clearenv', '--setenv', 'LANG', 'C.UTF-8',
        '--setenv', 'TZ', 'UTC', '--setenv', 'FRAMERIZE_PARENT_NETNS',
        os.readlink('/proc/self/ns/net'), '--setenv', 'FRAMERIZE_PARENT_PIDNS',
        os.readlink('/proc/self/ns/pid'), '--ro-bind', '/usr', '/usr',
        '--symlink', 'usr/bin', '/bin', '--symlink', 'usr/lib', '/lib',
        '--symlink', 'usr/lib64', '/lib64', '--proc', '/proc', '--dev', '/dev',
        '--tmpfs', '/tmp', '--ro-bind', '/sys/fs/cgroup', '/sys/fs/cgroup']
    for path in [settings['node'], settings['dom_root'], *settings['files']]:
        argv += ['--ro-bind', path, path]
    argv += ['--ro-bind', str(source), '/work', '--bind', str(output), '/out',
             '--chdir', '/work', '/work/owner-probe']
    return argv


def exercise(lab, source, output, result):
    state = lab.state('host', selected_device='synthetic-owner-measurement')
    child = lab.start(state)
    account = state['accounts'][0]
    status = lab.command(child, 'browser_status', account=account)
    generation = status['host_generation']
    assert (status['state'], status['active_commands']) == ('absent', 0)
    result.update(host_generation=generation, host_pid=child.pid)
    restored = json.loads((state['root'] / 'cooldown-handoff' /
        ('restored-' + state['nonce'] + '.json')).read_text())
    assert restored['generation'] == generation and restored['main'] == lab.parent
    assert restored['request_sha256'] == state['pending_hash']
    assert restored['snapshot_sha256'] == state['snapshot_hash']
    scope = dict(account=account, generation=generation, lease=str(uuid.uuid4()))
    opened = lab.command(child, 'native_device_open', **scope)
    assert opened == dict(generation=generation, lease=scope['lease'],
                          device='synthetic-owner-measurement')
    ticket = lab.command(child, 'native_device_mint', **scope, budget_ms=16000)
    assert ticket == dict(generation=generation, lease=scope['lease'], sequence=1)
    result['lease_before_worker'] = check_status(lab, child, account, generation, 1)
    result['phases']['lease_held'] = sample(group_root())
    worker_output = output / 'worker'
    worker_output.mkdir(mode=0o700)
    command = worker_command(source, worker_output)
    started = time.monotonic()
    worker = subprocess.run(command, stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=25)
    result['worker_command'] = command
    result['worker_wait_ms'] = (time.monotonic() - started) * 1000
    result['worker_exit_code'] = worker.returncode
    assert worker.returncode == 0, 'owned worker failed'
    minted = json.loads((worker_output / 'result.json').read_text())
    assert all(minted.get(key) is True for key in (
        'accepted', 'synthetic', 'worker_joined', 'worker_absent', 'token_discarded'))
    assert minted.get('authenticated_delivery') is False
    result.update(worker_result=minted, worker_result_sha256=sha(worker_output / 'result.json'))
    result['lease_after_worker'] = check_status(lab, child, account, generation, 1)
    result['phases']['worker_joined_lease_held'] = sample(group_root())
    confirmed = lab.command(child, 'native_device_minted', **scope, sequence=1, joined=True)
    assert confirmed == dict(confirmed=True, ready=True)
    result['mint_confirmed'] = confirmed
    closed = lab.command(child, 'native_device_close', **scope, joined=True, risk_refused=False)
    assert closed == dict(confirmed=True)
    result['lease_released'] = check_status(lab, child, account, generation, 0)
    again = dict(scope, lease=str(uuid.uuid4()))
    assert lab.command(child, 'native_device_open', **again) == dict(
        generation=generation, lease=again['lease'], device='synthetic-owner-measurement')
    check_status(lab, child, account, generation, 1)
    assert lab.command(child, 'native_device_close', **again,
        joined=True, risk_refused=False) == dict(confirmed=True)
    result['fresh_lease_released'] = check_status(lab, child, account, generation, 0)
    result['fresh_lease_without_second_mint'] = True
    result['phases']['host_idle'] = sample(group_root())


def isolated(root, source, group):
    assert root.is_absolute() and root.is_dir() and not any(root.iterdir())
    assert source.is_absolute() and source.is_dir()
    assert os.readlink('/proc/self/ns/net') != os.environ['FRAMERIZE_PARENT_NETNS']
    for path in ('.framerize/framerize.db', '.framerize/profiles',
                 '.local/state/framerize-browserless'):
        assert not (Path('/home/tjninja') / path).exists(), 'account state visible'
    assert not Path('/run/user/' + str(os.getuid()) + '/bus').exists()
    assert int((group / 'memory.max').read_text()) == 128 * 1024 ** 2
    assert int((group / 'memory.swap.max').read_text()) == 0
    assert int((group / 'pids.max').read_text()) == 32


def main():
    root, package, python, worker_source = map(Path, sys.argv[1:])
    os.umask(0o077)
    group = group_root()
    lab = None
    result = dict(accepted=False, synthetic=True, authenticated_delivery=False,
        external_network=False, real_accounts=False, application_daemon=False,
        started_at_unix=time.time(), cgroup=str(group), phases={},
        scope='One host lease and one owned synthetic mint; no HTTP or inference.')
    try:
        isolated(root, worker_source, group)
        result['phases']['before_host'] = sample(group)
        lab = Lab(root, package, python)
        exercise(lab, worker_source, root, result)
        result['workload_passed'] = True
    except Exception as error:
        result['error_type'] = type(error).__name__
    finally:
        if lab is not None:
            try:
                lab.finish()
            except Exception as error:
                result['cleanup_error'] = type(error).__name__
            result['process_joins'] = lab.joins
            result['all_owned_processes_joined'] = not lab.children
    result['resources'] = sample(group)
    result['memory_events'] = dict((key, int(value)) for key, value in (
        line.split() for line in (group / 'memory.events').read_text().splitlines()))
    result['unexpected_processes'] = sorted(set((group / 'cgroup.procs').read_text().split())
        - {str(os.getpid()), str(os.getppid())})
    result['accepted'] = (
        result.get('workload_passed') is True
        and result.get('all_owned_processes_joined') is True
        and 'cleanup_error' not in result
        and not any(join['forced'] for join in result.get('process_joins', []))
        and not result['unexpected_processes']
        and not any(result['memory_events'].values())
        and result['resources']['memory.swap.current'] == 0
        and result['resources']['memory.swap.peak'] == 0)
    result['finished_at_unix'] = time.time()
    with (root / 'result.json').open('x') as stream:
        json.dump(result, stream, indent=2, sort_keys=True)
        stream.write('\n')
    print(json.dumps(dict(accepted=result['accepted'], resources=result['resources'])))
    return 0 if result['accepted'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
