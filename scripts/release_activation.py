"""Activate an already staged release of the managed browser service."""
import fcntl
from contextlib import closing
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import sqlite3
import subprocess
import tempfile
import time
import urllib.request
import uuid

from release import verify


def _sync(directory):
    fd = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def _switch(current, target):
    temporary = current.with_name('.current-' + uuid.uuid4().hex)
    try:
        temporary.symlink_to(target)
        temporary.replace(current)
        _sync(current.parent)
    finally:
        temporary.unlink(missing_ok=True)


def _release(path, installation):
    path = path.resolve(strict=True)
    if path.parent != installation / 'releases':
        raise ValueError('release must be staged inside this installation')
    info = verify(path)
    if (info.get('application') != 'framerize' or info.get('development') is not False
            or info.get('source_dirty') is not False or info.get('build_profile') != 'release'
            or type(info.get('rate_history_schema')) is not int
            or type(info.get('activation_protocol')) is not int or info['activation_protocol'] != 1):
        raise ValueError('activation requires a complete production browser package')
    if not os.access(path / 'bin/framerize', os.X_OK):
        raise ValueError('packaged application is not executable')
    return path, info


def _preflight(old, new, installation, profiles, service):
    old, before = _release(old, installation)
    new, after = _release(new, installation)
    if before['rate_history_schema'] != after['rate_history_schema']:
        raise ValueError('automatic rollback requires matching history schemas')
    if before.get('platform') != after.get('platform'):
        raise ValueError('package platforms differ')
    history = profiles / '.sidecar/camoufox/rate-history.sqlite3'
    if not history.is_file():
        raise ValueError('existing account history requires migration before activation')
    with closing(sqlite3.connect(history.as_uri() + '?mode=ro', uri=True)) as db:
        schema = db.execute('PRAGMA user_version').fetchone()[0]
    if schema != after['rate_history_schema']:
        raise ValueError('account history schema requires a separate migration')
    from release_checkpoint_history import validate as validate_checkpoints
    validate_checkpoints(before, after, profiles)
    service.validate(old, new)
    return old, new


def _intent(path, old, new):
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(mode='w', dir=path.parent, delete=False) as stream:
            temporary = Path(stream.name)
            json.dump({'version': 1, 'old': str(old), 'new': str(new)}, stream)
            stream.flush()
            os.fsync(stream.fileno())
        temporary.replace(path)
        _sync(path.parent)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def _clear(path):
    path.unlink()
    _sync(path.parent)


def _restore(current, old, service, pending):
    service.stop(current.resolve() / 'bin/framerize')
    _switch(current, old)
    service.start(old)
    _clear(pending)


def activate(package, installation, profiles, service, *, qualification=None):
    installation, profiles = installation.resolve(), profiles.resolve()
    current = installation / 'current'
    if not current.is_symlink():
        raise ValueError('managed-service migration is required before activation')
    fd = os.open(installation / '.update.lock', os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return _activate_locked(package, installation, profiles, service, qualification)


def _activate_locked(package, installation, profiles, service, qualification=None):
    current, pending = installation / 'current', installation / 'update.json'
    if pending.exists():
        record = json.loads(pending.read_text())
        if (type(record) is not dict or set(record) != {'version', 'old', 'new'}
                or type(record['version']) is not int or record['version'] != 1):
            raise ValueError('unrecognized interrupted update record')
        old, new = _preflight(Path(record['old']), Path(record['new']),
                              installation, profiles, service)
        if current.resolve() not in (old, new):
            raise ValueError('current release differs from the interrupted update')
        _restore(current, old, service, pending)
        return {'status': 'recovered', 'release': str(old)}
    old, new = _preflight(current.resolve(), package, installation, profiles, service)
    if old == new:
        service.start(old)
        return {'status': 'unchanged', 'release': str(old)}
    from release_qualification import validate
    validate(new, qualification)
    _intent(pending, old, new)
    try:
        service.stop(old / 'bin/framerize')
        _switch(current, new)
        service.start(new)
    except BaseException as error:
        try:
            _restore(current, old, service, pending)
        except BaseException as rollback:
            raise RuntimeError('activation and rollback failed; update.json retains recovery state') from rollback
        raise RuntimeError('activation failed; previous release restored') from error
    _clear(pending)
    return {'status': 'activated', 'release': str(new)}


class UserService:
    def __init__(self):
        self.unit = 'framerize.service'
        self.unit_file = Path.home() / '.config/systemd/user' / self.unit
        self.http = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def command(self, *args):
        return subprocess.run(['/usr/bin/systemctl', '--user', '--no-pager', *args],
            check=True, capture_output=True, text=True, timeout=330,
            env={'PATH': '/usr/bin:/bin', 'XDG_RUNTIME_DIR': f'/run/user/{os.getuid()}'})

    def show(self):
        fields = ('LoadState,ActiveState,MainPID,FragmentPath,DropInPaths,KillMode,SendSIGKILL,Delegate,NeedDaemonReload,'
                  'ExecStart,Restart,RestartUSec,TimeoutStopUSec,MemoryMax,MemoryHigh,MemorySwapMax,TasksMax,'
                  'CPUQuotaPerSecUSec,CPUQuotaPeriodUSec,StartLimitIntervalUSec,StartLimitBurst')
        result = self.command('show', '--property=' + fields, '--', self.unit)
        return dict(line.split('=', 1) for line in result.stdout.splitlines())

    @staticmethod
    def _configuration_snapshot(info, unit_file):
        paths = [unit_file, *(Path(path) for path in info.get('DropInPaths', '').split())]
        files = []
        for path in paths:
            if path.is_symlink() or not path.is_file():
                raise ValueError('service configuration contains a missing file or symbolic link')
            files.append((str(path.resolve()), hashlib.sha256(path.read_bytes()).hexdigest()))
        start = info.get('ExecStart', '')
        path = re.search(r'(?:^|[\s{])path=([^ ;}]+)', start)
        argv = re.search(r'(?:^|[\s{])argv\[\]=([^;}]+)', start)
        if not path or not argv:
            raise ValueError('service has no single inspectable executable command')
        properties = ('LoadState', 'FragmentPath', 'DropInPaths', 'KillMode', 'SendSIGKILL', 'Delegate',
            'NeedDaemonReload', 'Restart', 'RestartUSec', 'TimeoutStopUSec', 'MemoryMax', 'MemoryHigh',
            'MemorySwapMax', 'TasksMax', 'CPUQuotaPerSecUSec', 'CPUQuotaPeriodUSec',
            'StartLimitIntervalUSec', 'StartLimitBurst')
        return {
            'files': tuple(files),
            'exec_start_path': path.group(1).strip(),
            'exec_start_argv_sha256': hashlib.sha256(argv.group(1).strip().encode()).hexdigest(),
            'properties': tuple((key, info.get(key, '')) for key in properties),
        }

    def verify_configuration(self):
        expected = getattr(self, '_validated_configuration', None)
        if expected is not None and self._configuration_snapshot(self.show(), self.unit_file) != expected:
            raise RuntimeError('service configuration changed after preflight')

    @staticmethod
    def executable(pid):
        return Path(os.readlink(f'/proc/{pid}/exe'))

    def validate(self, old, new):
        info = self.show()
        template = Path('share/systemd/user/framerize.service')
        if (self.unit_file.is_symlink() or not self.unit_file.is_file()
                or (old / template).read_text().splitlines() != (new / template).read_text().splitlines()
                or info.get('LoadState') != 'loaded'
                or info.get('NeedDaemonReload') != 'no'
                or Path(info.get('FragmentPath', '')).resolve() != self.unit_file.resolve()):
            raise ValueError('installed service requires migration; refusing to replace it')
        if (info.get('KillMode') not in ('mixed', 'control-group')
                or info.get('SendSIGKILL') != 'yes' or info.get('Delegate') != 'no'
                or info.get('ActiveState') not in ('active', 'inactive', 'failed')):
            raise ValueError('service lifecycle configuration differs')
        configuration = self._configuration_snapshot(info, self.unit_file)
        expected = str(Path.home() / '.local/share/framerize/current/bin/framerize')
        if configuration['exec_start_path'] != expected:
            raise ValueError('service executable does not use the managed current release')
        pid = int(info.get('MainPID', '0'))
        if info['ActiveState'] == 'active' and not pid:
            raise ValueError('active service has no main process')
        if pid and self.executable(pid) not in (old / 'bin/framerize', new / 'bin/framerize'):
            raise ValueError('running executable does not belong to this update')
        self._validated_configuration = configuration

    def stop(self, executable):
        self.verify_configuration()
        from release_drain import descendants, state, stop
        info = state(self)
        if info.get('ActiveState') == 'active':
            return stop(self, executable)
        if info.get('ActiveState') in ('inactive', 'failed') and info.get('MainPID') == '0':
            if descendants(info.get('ControlGroup', '')):
                raise RuntimeError('stopped service still has processes in its cgroup')
            if state(self) != info:
                raise RuntimeError('service state changed while confirming it was stopped')
            return {'already_stopped': True, 'remaining_descendants': 0}
        raise RuntimeError('service is not in a drainable state')

    def start(self, package):
        self.command('start', '--', self.unit)
        deadline = time.monotonic() + 90
        while time.monotonic() < deadline:
            info = self.show()
            if info.get('ActiveState') in ('inactive', 'failed'):
                raise RuntimeError('service exited before readiness')
            pid = int(info.get('MainPID', '0'))
            try:
                if pid and self.executable(pid) == package / 'bin/framerize':
                    from release_drain import require_readiness
                    try:
                        require_readiness(self, pid)
                        ready = True
                    except (OSError, RuntimeError, ValueError):
                        ready = False
                    after = self.show()
                    if ready and after.get('MainPID') == str(pid) and after.get('ActiveState') == 'active':
                        self.verify_configuration()
                        return
            except OSError:
                pass
            time.sleep(0.1)
        raise RuntimeError('service readiness timed out')


def run(package, qualification=None):
    def interrupted(*_):
        raise KeyboardInterrupt('activation interrupted')
    previous = signal.signal(signal.SIGTERM, interrupted)
    try:
        return activate(package, Path.home() / '.local/share/framerize',
                        Path.home() / '.framerize/profiles', UserService(),
                        qualification=qualification)
    finally:
        signal.signal(signal.SIGTERM, previous)
