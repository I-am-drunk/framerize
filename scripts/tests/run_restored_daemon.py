"""Exercise restored host ownership through the real isolated HTTP server."""
import json
import os
from pathlib import Path
import sys
import unittest

from restored_daemon import DaemonLab


class RestoredDaemon(unittest.TestCase):
    lab = None
    observations = {}

    def start(self, **kwargs):
        state = self.lab.state(self._testMethodName, **kwargs)
        child = self.lab.start_daemon(state)
        status, body, _ = self.lab.request(state, '/api/accounts')
        self.assertEqual(status, 200)
        self.assertEqual([row['id'] for row in json.loads(body)], state['accounts'])
        self.lab.no_children(child)
        self.observations[self._testMethodName] = dict(responses=[])
        return state, child

    def tearDown(self):
        self.lab.finish()

    def refused(self, state, expected):
        payload = dict(model='gpt-4.1', input='fixture only', stream=True, max_output_tokens=16)
        status, body, _ = self.lab.request(state, '/openai/v1/responses', payload)
        error = json.loads(body)['error']
        self.observations[self._testMethodName]['responses'].append(
            dict(status=status, code=error.get('code')))
        self.assertEqual(error.get('type'), 'native_dispatch_error')
        self.assertEqual((status, error.get('code')), expected)

    def idle(self, state):
        status, body, _ = self.lab.request(state, '/api/pool')
        self.assertEqual(status, 200)
        value = json.loads(body)
        self.assertEqual(value['config']['eligible_accounts'], state['accounts'])
        rows = value['residency']['residents']
        self.assertEqual(len(rows), 1)
        row = rows[0]
        self.assertEqual((row['account'], row['status'], row['users'], row['replacement'],
            row['uncertain'], row['close_started']),
            (state['accounts'][0], 'idle', 0, None, False, False))
        status, body, headers = self.lab.request(state, '/anthropic/api/hello', method='HEAD')
        self.assertEqual((status, body), (200, b''))
        self.assertEqual({key.lower(): value for key, value in headers.items()}['content-length'], '2')

    def test_missing_session_releases_restored_device_for_next_request(self):
        state, _ = self.start()
        self.refused(state, (502, 'Authorization'))
        first = self.lab.witness(state)
        self.idle(state)
        self.refused(state, (502, 'Authorization'))
        self.assertEqual(self.lab.witness(state), first)
        self.idle(state)
        self.lab.unchanged(state)
        self.observations[self._testMethodName]['restoration'] = first

    def test_restored_full_window_refuses_before_session_or_worker(self):
        state, _ = self.start(full=True)
        self.refused(state, (503, 'Ineligible'))
        self.observations[self._testMethodName]['restoration'] = self.lab.witness(state)
        self.idle(state)
        self.lab.unchanged(state)

    def test_changed_json_preserves_refusal_and_does_not_restore(self):
        state, child = self.start(changed=True)
        self.refused(state, (502, 'Cleanup'))
        directory = state['root'] / 'cooldown-handoff'
        self.assertFalse(list(directory.glob('claim-*.json')))
        self.assertFalse(list(directory.glob('restored-*.json')))
        self.assertIn('saved account state changed', (state['root'] / 'server.log').read_text())
        self.assertIsNone(child.poll())
        self.lab.unchanged(state)


def main():
    root, package, python, binary, routing = map(Path, sys.argv[1:6])
    selection = sys.argv[6:]
    allowed = unittest.defaultTestLoader.getTestCaseNames(RestoredDaemon)
    assert len(selection) == len(set(selection)) and all(name in allowed for name in selection)
    assert root.is_absolute() and root.is_dir() and not any(root.iterdir())
    assert os.readlink('/proc/self/ns/net') != os.environ['FRAMERIZE_PARENT_NETNS']
    assert not (Path.home() / '.framerize/framerize.db').exists()
    assert not (Path.home() / '.framerize/profiles').exists()
    assert not (Path.home() / '.local/state/framerize-browserless').exists()
    assert not Path('/run/user/' + str(os.getuid()) + '/bus').exists()
    os.umask(0o077)
    lab = None
    result = dict(accepted=False, real_accounts=False, external_network=False,
        scope='Real server and generated host with synthetic state; no SDK or authenticated inference.')
    try:
        lab = DaemonLab(root, package, python, binary, routing)
        RestoredDaemon.lab = lab
        suite = unittest.TestSuite(RestoredDaemon(name) for name in selection or allowed)
        result['case_names'] = [case.id() for case in suite]
        tests = unittest.TextTestRunner(stream=sys.stdout, verbosity=2).run(suite)
        result.update(tests_run=tests.testsRun, skipped=len(tests.skipped),
            failures=[case.id() for case, _ in tests.failures],
            errors=[case.id() for case, _ in tests.errors], accepted=tests.wasSuccessful())
    except Exception as error:
        result.update(accepted=False, error_type=type(error).__name__)
    finally:
        if lab is not None:
            try:
                lab.finish()
            except Exception as error:
                result.update(accepted=False, cleanup_error=type(error).__name__)
            result['process_joins'] = lab.joins
            result['all_owned_processes_joined'] = not lab.children and not lab.daemons
        result['observations'] = RestoredDaemon.observations
    group = Path('/sys/fs/cgroup' + Path('/proc/self/cgroup').read_text().strip().split('::', 1)[1])
    result['cgroup'] = str(group)
    result['resources'] = {name: int((group / name).read_text()) for name in
        ('memory.current', 'memory.peak', 'memory.swap.current', 'pids.current', 'pids.peak')}
    if (group / 'memory.swap.peak').exists():
        result['resources']['memory.swap.peak'] = int((group / 'memory.swap.peak').read_text())
    result['memory_events'] = dict(line.split() for line in (group / 'memory.events').read_text().splitlines())
    result['unexpected_processes'] = sorted(set((group / 'cgroup.procs').read_text().split())
        - {str(os.getpid()), str(os.getppid())})
    if (result['resources']['memory.swap.current'] != 0 or result['unexpected_processes']
            or any(int(value) != 0 for value in result['memory_events'].values())):
        result['accepted'] = False
    with (root / 'result.json').open('x') as stream:
        json.dump(result, stream, indent=2, sort_keys=True)
        stream.write('\n')
    print(json.dumps({'accepted': result['accepted'], 'tests_run': result.get('tests_run')}))
    return 0 if result['accepted'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
