"""Measure an isolated application and host during a separate synthetic mint."""
import json
import os
from pathlib import Path
import subprocess
import sys
import time

from restored_daemon import DaemonLab
from restored_native_host import sha
from run_host_worker import group_root, sample, worker_command
from application_memory_roles import ProcessRoles, record_attribution


def resident_bytes(pid):
    fields = dict(line.split(':', 1) for line in
                  Path('/proc', str(pid), 'status').read_text().splitlines())
    return int(fields['VmRSS'].split()[0]) * 1024


def process_memory(group, failures=None, classify=None):
    try:
        pids = set((group / 'cgroup.procs').read_text().split())
        totals = dict(pss_with_rss_fallback=0, rss=0, processes=len(pids), fallback_processes=0)
        if classify is not None:
            totals['roles'] = {}
        for pid in pids:
            before = classify(int(pid)) if classify is not None else None
            fallback = False
            try:
                lines = Path('/proc', pid, 'smaps_rollup').read_text().splitlines()
                fields = {line.split(':', 1)[0]: int(line.split()[1]) * 1024
                          for line in lines if line.startswith(('Rss:', 'Pss:'))}
            except PermissionError:
                rss = resident_bytes(int(pid))
                fields = {'Rss': rss, 'Pss': rss}
                totals['fallback_processes'] += 1
                fallback = True
            totals['pss_with_rss_fallback'] += fields['Pss']
            totals['rss'] += fields['Rss']
            if classify is not None:
                if before != classify(int(pid)):
                    if failures is not None:
                        failures['IdentityChanged'] = failures.get('IdentityChanged', 0) + 1
                    return None
                role = totals['roles'].setdefault(before[0], dict(
                    pss_with_rss_fallback=0, rss=0, processes=0, fallback_processes=0))
                role['pss_with_rss_fallback'] += fields['Pss']
                role['rss'] += fields['Rss']
                role['processes'] += 1
                role['fallback_processes'] += int(fallback)
        current_pids = set((group / 'cgroup.procs').read_text().split())
    except (OSError, KeyError, ValueError) as error:
        if failures is not None:
            name = type(error).__name__
            failures[name] = failures.get(name, 0) + 1
        return None
    if pids != current_pids:
        if failures is not None:
            failures['MembershipChanged'] = failures.get('MembershipChanged', 0) + 1
        return None
    return totals


def exercise(lab, source, root, result):
    state = lab.state('application')
    daemon = lab.start_daemon(state)
    payload = dict(model='gpt-4.1', input='fixture only', stream=True, max_output_tokens=16)
    status, body, _ = lab.request(state, '/openai/v1/responses', payload)
    error = json.loads(body)['error']
    assert (status, error.get('type'), error.get('code')) == (
        502, 'native_dispatch_error', 'Authorization')
    identity = lab.witness(state)
    lab.unchanged(state)
    result.update(application_identity=identity, local_setup_posts=1)
    group = group_root()
    result['phases']['application_and_host_ready'] = sample(group)
    classify = ProcessRoles(identity['main'], identity['sidecar'], source)
    result['baseline_memory_failures'] = {}
    result['baseline_process_memory'] = process_memory(
        group, result['baseline_memory_failures'], classify)
    assert result['baseline_process_memory'] is not None, 'baseline process-memory coverage'
    output = root / 'worker'
    output.mkdir(mode=0o700)
    command = worker_command(source, output)
    result['worker_command'] = command
    started = time.monotonic()
    worker = subprocess.Popen(command, stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    lab.children.append(worker)
    observed = dict(samples=0, group_bytes=0, application_rss=0, host_rss=0)
    observed.update(memory_samples=0, memory_skipped=0, process_pss_rss_peak=0,
                    process_rss_peak=0, process_count_peak=0, fallback_processes_peak=0)
    observed['sample_failures'] = {}
    observed['attribution'] = {}
    result['overlap_observation'] = observed
    try:
        while worker.poll() is None:
            assert daemon.poll() is None, 'application exited during mint'
            assert time.monotonic() - started < 25, 'worker deadline'
            values = {'group_bytes': sample(group)['memory.current'],
                'application_rss': resident_bytes(daemon.pid),
                'host_rss': resident_bytes(identity['sidecar']['pid'])}
            for key, value in values.items():
                observed[key] = max(observed[key], value)
            observed['samples'] += 1
            processes = process_memory(group, observed['sample_failures'], classify)
            if processes is None:
                observed['memory_skipped'] += 1
            else:
                observed['memory_samples'] += 1
                record_attribution(observed['attribution'], processes)
                for key, field in (('process_pss_rss_peak', 'pss_with_rss_fallback'),
                                   ('process_rss_peak', 'rss'), ('process_count_peak', 'processes'),
                                   ('fallback_processes_peak', 'fallback_processes')):
                    observed[key] = max(observed[key], processes[field])
            time.sleep(0.02)
        assert worker.returncode == 0 and observed['samples'] > 0, 'worker failed'
        assert observed['memory_samples'] > 0, 'no complete process-memory sample'
    finally:
        lab.stop(worker)
        result['worker_exit_code'] = worker.returncode
        result['worker_wait_ms'] = (time.monotonic() - started) * 1000
    minted = json.loads((output / 'result.json').read_text())
    assert all(minted.get(key) is True for key in (
        'accepted', 'synthetic', 'worker_joined', 'worker_absent', 'token_discarded'))
    assert minted.get('authenticated_delivery') is False
    assert lab.witness(state) == identity, 'application or host generation changed'
    assert lab.request(state, '/api/health')[:2] == (200, b'ok')
    lab.unchanged(state)
    result.update(worker_result=minted, worker_result_sha256=sha(output / 'result.json'),
                  application_alive_after_worker=True)
    result['phases']['worker_joined_application_alive'] = sample(group)


def isolated(root, source, group):
    assert root.is_absolute() and root.is_dir() and not any(root.iterdir())
    assert source.is_absolute() and source.is_dir()
    assert os.readlink('/proc/self/ns/net') != os.environ['FRAMERIZE_PARENT_NETNS']
    for name in ('.framerize/framerize.db', '.framerize/profiles',
                 '.local/state/framerize-browserless'):
        assert not (Path('/home/tjninja') / name).exists(), 'account state visible'
    assert not Path('/run/user/' + str(os.getuid()) + '/bus').exists()
    assert int((group / 'memory.max').read_text()) == 256 * 1024 ** 2
    assert int((group / 'memory.swap.max').read_text()) == 0
    assert int((group / 'pids.max').read_text()) == 32


def main():
    root, package, python, binary, routing, worker_source = map(Path, sys.argv[1:])
    os.umask(0o077)
    group, lab = group_root(), None
    result = dict(accepted=False, synthetic=True, authenticated_delivery=False,
        external_network=False, real_accounts=False, application_daemon=True,
        separate_worker_driver=True, started_at_unix=time.time(),
        cgroup=str(group), phases={}, target_100mb_proven=False,
        scope='Application and host alive during a separate owned synthetic mint; no HTTP inference.')
    try:
        isolated(root, worker_source, group)
        result['phases']['before_application'] = sample(group)
        lab = DaemonLab(root, package, python, binary, routing)
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
    result['events'] = {name: dict((key, int(value)) for key, value in (
        line.split() for line in (group / name).read_text().splitlines()))
        for name in ('memory.events', 'memory.swap.events', 'pids.events')}
    result['unexpected_processes'] = sorted(set((group / 'cgroup.procs').read_text().split())
        - {str(os.getpid()), str(os.getppid())})
    result['observed_group_below_100mb'] = result['resources']['memory.peak'] <= 100000000
    observed = result.get('overlap_observation', {})
    result['observed_pss_with_rss_fallback_below_100mb'] = (
        observed['process_pss_rss_peak'] <= 100000000 if observed.get('memory_samples') else None)
    result['memory_scope'] = (
        'Group accounting and sampled process PSS/RSS include measurement helpers. '
        'Unreadable detailed maps use RSS and are counted explicitly. '
        'Role attribution binds process starts and executable device/inode; '
        'unknown roles remain included in the product estimate. '
        'Process samples require stable membership but are not atomic or exact peaks.')
    result['accepted'] = (
        result.get('workload_passed') is True
        and result.get('all_owned_processes_joined') is True
        and 'cleanup_error' not in result
        and not any(join['forced'] for join in result.get('process_joins', []))
        and not result['unexpected_processes']
        and not any(value for event in result['events'].values() for value in event.values())
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
