"""One direct child and its descendants inside a delegated, bounded cgroup."""
import os
from pathlib import Path
import re
import socket
import subprocess
import time


class ProcessError(RuntimeError):
    def __init__(self):
        super().__init__('comparison process ownership failed')


def require(value):
    if not value:
        raise ProcessError()


class Child:
    def __init__(self, binary, environment, *, memory_bytes, tasks=96):
        require(type(memory_bytes) is int and 0 < memory_bytes <= 1536 * 1024**2)
        require(type(tasks) is int and 1 <= tasks <= 256)
        self.binary, self.environment = str(binary), dict(environment)
        self.memory_bytes, self.tasks = memory_bytes, tasks
        self.process = self.socket = self.group = None
        self.forced = False
        self.started = False

    def _read(self, name):
        return (self.group / name).read_text().strip()

    def launch(self):
        require(not self.started)
        self.started = True
        relative = Path('/proc/self/cgroup').read_text().strip()
        require(relative.startswith('0::/') and '\n' not in relative)
        parent = Path('/sys/fs/cgroup') / relative[4:]
        require(re.fullmatch(r'framerize-browser-comparison-[a-z0-9-]+\.service', parent.name))
        require(parent.resolve(strict=True) == parent)
        cap = (parent / 'memory.max').read_text().strip()
        require(cap.isdecimal() and self.memory_bytes < int(cap) <= 2 * 1024**3)
        require((parent / 'memory.swap.max').read_text().strip() == '0')
        require((parent / 'memory.swap.current').read_text().strip() == '0')
        parent_tasks = (parent / 'pids.max').read_text().strip()
        require(parent_tasks.isdecimal() and self.tasks < int(parent_tasks) <= 320)
        require(not any(int(v) for _, v in
                        (line.split() for line in (parent / 'memory.events').read_text().splitlines())))
        require(set((parent / 'cgroup.procs').read_text().split()) == {str(os.getpid())})
        supervisor = parent / 'comparison-supervisor'
        supervisor.mkdir(mode=0o700)
        (supervisor / 'cgroup.procs').write_text(str(os.getpid()))
        (parent / 'cgroup.subtree_control').write_text('+memory +pids')
        group = parent / 'comparison-child'
        group.mkdir(mode=0o700)
        self.group = group
        for key, value in [('memory.max', self.memory_bytes), ('memory.swap.max', 0),
                           ('pids.max', self.tasks), ('memory.oom.group', 1)]:
            (group / key).write_text(str(value))
        self.socket, child_socket = socket.socketpair()
        try:
            self.process = subprocess.Popen(
                [self.binary, '--control-fd', str(child_socket.fileno())],
                pass_fds=(child_socket.fileno(),), env=self.environment,
                stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL, start_new_session=True)
            (group / 'cgroup.procs').write_text(str(self.process.pid))
            require(Path(f'/proc/{self.process.pid}/cgroup').read_text().strip()
                    == relative + '/comparison-child')
        finally:
            child_socket.close()
        return self.socket

    def sample(self):
        require(self.group is not None)
        values = {key: int(self._read(key)) for key in (
            'memory.current', 'memory.peak', 'memory.max', 'memory.swap.current',
            'memory.swap.max', 'pids.current', 'pids.max', 'pids.peak')}
        for key in ('memory.events', 'pids.events'):
            values[key] = dict((k, int(v)) for k, v in
                               (line.split() for line in self._read(key).splitlines()))
        require(values['memory.max'] == self.memory_bytes)
        require(values['memory.swap.max'] == values['memory.swap.current'] == 0)
        require(values['pids.max'] == self.tasks)
        return values

    def _empty(self):
        return dict(line.split() for line in self._read('cgroup.events').splitlines())['populated'] == '0'

    def _wait_empty(self, seconds):
        until = time.monotonic() + seconds
        while not self._empty():
            if time.monotonic() >= until:
                return False
            time.sleep(0.02)
        return True

    def close(self):
        if self.socket is not None:
            self.socket.close()
            self.socket = None
        if self.process is not None:
            try:
                self.process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self.forced = True
        if self.group is not None and not self._wait_empty(2):
            self.forced = True
            (self.group / 'cgroup.kill').write_text('1')
        if self.process is not None and self.process.poll() is None:
            self.forced = True
            self.process.kill()  # Also covers failure to attach before any plan was sent.
        if self.process is not None:
            self.process.wait(timeout=3)
        facts = dict(exit_code=None if self.process is None else self.process.returncode,
                     forced=self.forced, group_absent=False, resources=None)
        if self.group is not None:
            require(self._wait_empty(3))
            facts['resources'] = self.sample()
            self.group.rmdir()
            facts['group_absent'] = not self.group.exists()
            require(facts['group_absent'])
        return facts
