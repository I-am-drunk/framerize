"""Observe one request made by the existing, hash-verified client checker."""
import argparse
import hashlib
import sys
import threading
import time
import types
from pathlib import Path


class MemorySampler:
    def __init__(self, group):
        self.group = Path(group)
        self.identity = None
        self.started = time.monotonic()
        self.samples = []
        self.error = None
        self.stop = threading.Event()
        self.thread = None

    def sample(self):
        before = self.group.stat()
        identity = (before.st_dev, before.st_ino)
        if self.identity is not None and identity != self.identity:
            raise RuntimeError('cgroup_changed')
        values = []
        for name in ('memory.current', 'memory.swap.current', 'pids.current'):
            with (self.group / name).open() as stream:
                value = int(stream.read(128))
            if value < 0:
                raise ValueError('negative_counter')
            values.append(value)
        after = self.group.stat()
        if (after.st_dev, after.st_ino) != identity:
            raise RuntimeError('cgroup_changed')
        self.identity = identity
        elapsed = round(time.monotonic() - self.started, 6)
        self.samples.append([elapsed, *values])

    def start(self):
        self.sample()
        self.thread = threading.Thread(target=self.poll, daemon=True)
        self.thread.start()

    def poll(self):
        while not self.stop.wait(0.1):
            if len(self.samples) >= 802:
                self.error = 'sample_limit'
                return
            try:
                self.sample()
            except Exception as error:
                self.error = type(error).__name__
                return

    def finish(self):
        self.stop.set()
        if self.thread is not None and self.thread.ident is not None:
            self.thread.join(timeout=1)
        joined = self.thread is None or not self.thread.is_alive()
        if len(self.samples) >= 802 and self.error is None:
            self.error = 'sample_limit'
        if joined and self.error is None and len(self.samples) < 802:
            try:
                self.sample()
            except (OSError, ValueError, RuntimeError) as error:
                self.error = type(error).__name__
        rows = list(self.samples)
        gaps = [right[0] - left[0] for left, right in zip(rows, rows[1:])]
        return dict(
            complete=joined and self.error is None,
            sampler_joined=joined, error=self.error if joined else 'sampler_join_timeout',
            interval_seconds=0.1, sample_count=len(rows),
            columns=['elapsed_seconds', 'memory_bytes', 'swap_bytes', 'tasks'],
            samples=rows, sampled_peak_bytes=max((row[1] for row in rows), default=None),
            maximum_gap_seconds=round(max(gaps, default=0), 6),
            scope='Shared service cgroup, including concurrent work. Sampled peak is a lower bound; no savings attribution.')


def observe_exchange(checker, original, observer_pin, checker_pin):
    def exchange(raw, route, report):
        service = report['service']
        group = Path('/sys/fs/cgroup' + service['cgroup'])
        sampler = MemorySampler(group)
        observation = dict(observer=observer_pin, checker=checker_pin)
        report['resource_observation'] = observation
        try:
            try:
                sampler.start()
            except checker.Refused:
                raise
            except Exception as error:
                sampler.error = type(error).__name__
                raise checker.Refused('resource_observation_unavailable') from None
            return original(raw, route, report)
        finally:
            observation.update(sampler.finish())
            readers = (
                ('memory_after', lambda: checker.memory_snapshot(service)),
                ('service_unchanged', lambda: checker.current_service() == service),
            )
            for name, read in readers:
                try:
                    observation[name] = read()
                except checker.Refused:
                    raise
                except Exception as error:
                    observation['complete'] = False
                    observation[name + '_error'] = type(error).__name__
            if observation.get('service_unchanged') is not True:
                observation['complete'] = False
    return exchange


def load_checker(path, expected):
    path = Path(path).resolve(strict=True)
    with path.open('rb') as stream:
        source = stream.read(65537)
    actual = hashlib.sha256(source).hexdigest()
    if len(source) > 65536 or actual != expected:
        raise ValueError('checker_source_mismatch')
    module = types.ModuleType('verified_browserless_client_check')
    module.__file__ = str(path)
    exec(compile(source, str(path), 'exec'), module.__dict__)
    return module, dict(path=str(path), sha256=actual)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--checker', type=Path, required=True)
    parser.add_argument('--checker-sha256', required=True)
    parser.add_argument('--execute', action='store_true', required=True)
    args = parser.parse_args()
    checker, checker_pin = load_checker(args.checker, args.checker_sha256)
    source = Path(__file__).resolve()
    observer_pin = dict(path=str(source), sha256=hashlib.sha256(source.read_bytes()).hexdigest())
    checker.exchange = observe_exchange(checker, checker.exchange, observer_pin, checker_pin)
    previous = sys.argv
    try:
        sys.argv = [str(args.checker), '--execute']
        return checker.main()
    finally:
        sys.argv = previous


if __name__ == '__main__':
    raise SystemExit(main())
