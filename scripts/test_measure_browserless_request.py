import hashlib
import importlib.util
import json
import signal
import tempfile
import threading
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

spec = importlib.util.spec_from_file_location(
    'measurement', Path(__file__).with_name('measure_browserless_request.py'))
measurement = importlib.util.module_from_spec(spec)
spec.loader.exec_module(measurement)


class Refused(Exception):
    pass


def counters(group, memory):
    group.mkdir(exist_ok=True)
    for name, value in [('memory.current', memory), ('memory.swap.current', 0),
                        ('pids.current', 3)]:
        temporary = group / (name + '.new')
        temporary.write_text(str(value))
        temporary.replace(group / name)


class MeasurementTests(unittest.TestCase):
    def test_peak_between_endpoints_is_retained_and_sampler_is_joined(self):
        with tempfile.TemporaryDirectory() as folder:
            group = Path(folder) / 'service'
            counters(group, 64)
            sampler = measurement.MemorySampler(group)
            peak_seen = threading.Event()
            sample = sampler.sample
            def observe():
                sample()
                if sampler.samples[-1][1] == 200:
                    peak_seen.set()
            sampler.sample = observe
            try:
                sampler.start()
                counters(group, 200)
                self.assertTrue(peak_seen.wait(2), sampler.error)
                counters(group, 16)
            finally:
                result = sampler.finish()
        self.assertTrue(result['complete'])
        self.assertTrue(result['sampler_joined'])
        self.assertEqual(result['samples'][0][1], 64)
        self.assertEqual(result['samples'][-1][1], 16)
        self.assertEqual(result['sampled_peak_bytes'], 200)

    def test_failed_request_keeps_final_measurement_and_is_not_retried(self):
        service = {'cgroup': '/unused-test-service', 'pid': 7}
        checker = SimpleNamespace(Refused=Refused,
            current_service=lambda: service, memory_snapshot=lambda _: {'memory.current': '32'})
        send = Mock(side_effect=OSError('private request diagnostic'))
        report = {'service': service}
        with tempfile.TemporaryDirectory() as folder:
            group = Path(folder) / 'service'
            counters(group, 32)
            sampler_type = measurement.MemorySampler
            with patch.object(measurement, 'MemorySampler', side_effect=lambda _: sampler_type(group)):
                exchange = measurement.observe_exchange(checker, send, {}, {})
                with self.assertRaises(OSError):
                    exchange(b'private request', '/route', report)
        send.assert_called_once_with(b'private request', '/route', report)
        observed = report['resource_observation']
        self.assertTrue(observed['sampler_joined'])
        self.assertEqual(observed['memory_after']['memory.current'], '32')
        self.assertTrue(observed['service_unchanged'])
        self.assertNotIn('private', json.dumps(report))

    def test_replaced_cgroup_refuses_before_the_request(self):
        service = {'cgroup': '/unused-test-service', 'pid': 7}
        checker = SimpleNamespace(Refused=Refused,
            current_service=lambda: service, memory_snapshot=lambda _: {})
        send, report = Mock(), {'service': service}
        with tempfile.TemporaryDirectory() as folder:
            group = Path(folder) / 'service'
            counters(group, 64)
            sampler = measurement.MemorySampler(group)
            sampler.sample()
            group.rename(Path(folder) / 'old-service')
            counters(group, 128)
            with patch.object(measurement, 'MemorySampler', return_value=sampler):
                exchange = measurement.observe_exchange(checker, send, {}, {})
                with self.assertRaisesRegex(Refused, 'resource_observation_unavailable'):
                    exchange(b'{}', '/route', report)
        send.assert_not_called()
        self.assertFalse(report['resource_observation']['complete'])
        self.assertTrue(report['resource_observation']['sampler_joined'])

    def test_deadline_during_final_measurements_is_preserved(self):
        def expire(_number, _frame):
            raise Refused('client_deadline')
        previous = signal.signal(signal.SIGALRM, expire)
        self.addCleanup(signal.signal, signal.SIGALRM, previous)
        for site in ('sample', 'memory_after'):
            with self.subTest(site=site), tempfile.TemporaryDirectory() as folder:
                group = Path(folder) / 'service'
                counters(group, 32)
                sampler = measurement.MemorySampler(group)
                sample, sent = sampler.sample, {'done': False}
                def final_sample():
                    if site == 'sample' and sent['done']:
                        signal.raise_signal(signal.SIGALRM)
                    sample()
                sampler.sample = final_sample
                sampler.poll = lambda: sampler.stop.wait(2)
                service = {'cgroup': '/unused-test-service', 'pid': 7}
                def final_memory(_service):
                    if site == 'memory_after':
                        signal.raise_signal(signal.SIGALRM)
                    return {'memory.current': '32'}
                checker = SimpleNamespace(Refused=Refused,
                    current_service=lambda: service, memory_snapshot=final_memory)
                def send(*_args):
                    sent['done'] = True
                with patch.object(measurement, 'MemorySampler', return_value=sampler):
                    exchange = measurement.observe_exchange(checker, send, {}, {})
                    with self.assertRaisesRegex(Refused, 'client_deadline'):
                        exchange(b'{}', '/route', {'service': service})
                self.assertTrue(sent['done'])
                self.assertFalse(sampler.thread.is_alive())

    def test_wrong_source_hash_is_rejected_before_checker_code_runs(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / 'check.py'
            path.write_text("raise RuntimeError('checker executed')\n")
            with self.assertRaisesRegex(ValueError, 'checker_source_mismatch'):
                measurement.load_checker(path, hashlib.sha256(b'other').hexdigest())


if __name__ == '__main__':
    unittest.main(verbosity=2)
