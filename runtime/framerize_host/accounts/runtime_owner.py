"""Recognize a fresh, exclusively occupied systemd user-service lifetime."""
import os
from pathlib import Path, PurePosixPath
import re
import subprocess

PROC = Path('/proc')
CGROUP = Path('/sys/fs/cgroup')
FIELDS = {'boot_id', 'unit', 'cgroup', 'invocation'}


def _group(pid):
    lines = (PROC / str(pid) / 'cgroup').read_text().splitlines()
    if len(lines) != 1 or not lines[0].startswith('0::/'):
        raise ValueError('unified cgroup required')
    group = lines[0][3:]
    path = PurePosixPath(group)
    if '..' in path.parts or str(path) != group or not path.name.endswith('.service'):
        raise ValueError('service cgroup required')
    return group


def _service(unit):
    fields = 'MainPID,ControlGroup,InvocationID,Delegate,KillMode,SendSIGKILL'
    result = subprocess.run(['/usr/bin/systemctl', '--user', '--no-pager',
        'show', '--property=' + fields, '--', unit], check=True,
        capture_output=True, text=True, timeout=5,
        env={'PATH': '/usr/bin:/bin', 'XDG_RUNTIME_DIR': f'/run/user/{os.getuid()}'})
    return dict(line.split('=', 1) for line in result.stdout.splitlines())


def capture():
    """Only the daemon and this host may exist before browser startup."""
    try:
        pid, parent = os.getpid(), os.getppid()
        group = _group(pid)
        if _group(parent) != group:
            return None
        unit = PurePosixPath(group).name
        info = _service(unit)
        if (info.get('MainPID') != str(parent) or info.get('ControlGroup') != group
                or info.get('Delegate') != 'no' or info.get('SendSIGKILL') != 'yes'
                or info.get('KillMode') not in ('control-group', 'mixed')
                or not re.fullmatch('[0-9a-f]{32}', info.get('InvocationID', ''))):
            return None
        directory = CGROUP / group.lstrip('/')
        for _ in range(2):
            if any(path.is_dir() for path in directory.iterdir()):
                return None
            members = {int(value) for value in (directory / 'cgroup.procs').read_text().split()}
            if members != {pid, parent} or os.getppid() != parent:
                return None
        boot = (PROC / 'sys/kernel/random/boot_id').read_text().strip()
        if not boot:
            return None
        return dict(boot_id=boot, unit=unit, cgroup=group, invocation=info['InvocationID'])
    except (OSError, ValueError, subprocess.SubprocessError):
        return None


def previous_stopped(previous, current, boot):
    return (type(previous) is dict and type(current) is dict
        and set(previous) == FIELDS and set(current) == FIELDS
        and previous['boot_id'] == current['boot_id'] == boot
        and previous['unit'] == current['unit'] and previous['cgroup'] == current['cgroup']
        and type(previous['invocation']) is str
        and re.fullmatch('[0-9a-f]{32}', previous['invocation']) is not None
        and previous['invocation'] != current['invocation'])
