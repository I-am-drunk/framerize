"""Drain a verified browser service without arming systemd's forced-stop timer."""
import os
import json
from pathlib import Path
import select
import signal

FIELDS = ('MainPID,InvocationID,ActiveState,SubState,Result,ExecMainCode,'
          'ExecMainStatus,Restart,NRestarts,ControlGroup,NeedDaemonReload')


def state(service):
    result = service.command('show', '--property=' + FIELDS, '--', service.unit)
    return dict(line.split('=', 1) for line in result.stdout.splitlines())


def descendants(group):
    if not group.startswith('/') or '..' in Path(group).parts:
        raise ValueError('invalid service cgroup')
    root = Path('/sys/fs/cgroup') / group.lstrip('/')
    def failed(error):
        if not isinstance(error, FileNotFoundError):
            raise error
    processes = set()
    for directory, _, _ in os.walk(root, onerror=failed):
        try:
            processes.update(map(int, (Path(directory) / 'cgroup.procs').read_text().split()))
        except FileNotFoundError:
            if Path(directory).exists():
                raise
    return processes


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def require_readiness(service, pid):
    for endpoint in ('health', 'ready'):
        with service.http.open('http://127.0.0.1:8787/api/' + endpoint, timeout=10) as reply:
            require(reply.status == 200 and reply.headers.get('x-framerize-pid') == str(pid),
                    'ownership readiness does not belong to the expected healthy process')
            if endpoint == 'ready':
                require(json.load(reply).get('ready') is True,
                        'the serving ownership or account-history gate is unavailable')


def clean_exit(info, before):
    require(info['MainPID'] == '0' and info['ActiveState'] == 'inactive',
            'draining service has not reached inactive state')
    require(info['Result'] == 'success' and info['ExecMainCode'] == '1'
            and info['ExecMainStatus'] == '0', 'service did not exit cleanly')
    require(info['NRestarts'] == before['NRestarts'],
            'service restarted during drain')
    require(info['InvocationID'] in ('', before['InvocationID']),
            'service invocation changed during drain')
    require(not descendants(before['ControlGroup']),
            'service descendants remain after drain')


def stop(service, executable):
    before = state(service)
    require(before['ActiveState'] == 'active' and int(before['MainPID']) > 0,
            'graceful drain requires the active serving process')
    require(before['Restart'] in ('no', 'on-failure')
            and before['NeedDaemonReload'] == 'no',
            'service cannot drain with these lifecycle settings')
    pid = int(before['MainPID'])
    descriptor = os.pidfd_open(pid)
    try:
        require(state(service) == before and service.executable(pid) == executable,
                'service identity changed before graceful drain')
        require_readiness(service, pid)
        signal.pidfd_send_signal(descriptor, signal.SIGTERM)
        # SIGTERM seals the application's admission gate. The pidfd wait does
        # not cancel requests or impose a deadline on their existing ownership.
        select.select([descriptor], [], [])
    finally:
        os.close(descriptor)
    after = state(service)
    clean_exit(after, before)
    return {'pid': pid, 'invocation': before['InvocationID'],
            'clean_exit': True, 'remaining_descendants': 0}
