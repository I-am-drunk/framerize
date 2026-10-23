"""Exercise the actual generated host with synthetic restored account state."""
import json
import os
from pathlib import Path
import sqlite3
import sys
import unittest
import uuid

from restored_native_host import Lab, sha


class RestoredHost(unittest.TestCase):
    lab = None

    def child(self, state, name='host'):
        child = self.lab.start(state, name)
        self.addCleanup(lambda: child in self.lab.children and self.lab.stop(child))
        return child

    def ready(self, counts=(0,), cooling=()):
        state = self.lab.state(self._testMethodName, counts, cooling)
        child = self.child(state)
        status = self.lab.command(child, 'browser_status', account=state['accounts'][0])
        self.assertEqual((status['state'], status['active_commands']), ('absent', 0))
        directory = state['root'] / 'cooldown-handoff'
        receipt = json.loads((directory / ('restored-' + state['nonce'] + '.json')).read_text())
        self.assertEqual(receipt['request_sha256'], state['pending_hash'])
        self.assertEqual(receipt['snapshot_sha256'], state['snapshot_hash'])
        self.assertEqual(receipt['generation'], status['host_generation'])
        self.assertEqual(receipt['main'], self.lab.parent)
        self.assertEqual(receipt['account_count'], len(counts))
        return state, child, status['host_generation']

    def scope(self, state, generation, index=0):
        return dict(account=state['accounts'][index], generation=generation, lease=str(uuid.uuid4()))

    def test_restored_process_uses_actual_selected_device_without_browser(self):
        state, child, generation = self.ready()
        scope = self.scope(state, generation)
        opened = self.lab.command(child, 'native_device_open', **scope)
        self.assertEqual(opened, dict(generation=generation, lease=scope['lease'], device='fixture-selected'))
        status = self.lab.command(child, 'browser_status', account=scope['account'])
        self.assertEqual((status['state'], status['active_commands']), ('absent', 1))
        closed = self.lab.command(child, 'native_device_close', **scope, joined=True, risk_refused=False)
        self.assertEqual(closed, dict(confirmed=True))
        status = self.lab.command(child, 'browser_status', account=scope['account'])
        self.assertEqual((status['state'], status['active_commands']), ('absent', 0))

    def test_restored_cooling_and_full_window_refuse_native_open(self):
        state, child, generation = self.ready((180, 0), cooling=(1,))
        for index in range(2):
            scope = self.scope(state, generation, index)
            self.assertEqual(self.lab.command(child, 'native_device_open', **scope), dict(rejected=True))
            status = self.lab.command(child, 'browser_status', account=scope['account'])
            self.assertEqual((status['state'], status['active_commands']), ('absent', 0))

    def test_native_charge_survives_confirmed_cleanup(self):
        state, child, generation = self.ready((179,))
        scope = self.scope(state, generation)
        self.assertNotIn('rejected', self.lab.command(child, 'native_device_open', **scope))
        self.assertEqual(self.lab.command(child, 'native_device_mint', **scope, budget_ms=1000),
            dict(generation=generation, lease=scope['lease'], sequence=1))
        self.assertEqual(self.lab.command(child, 'native_device_minted', **scope, sequence=1, joined=True),
            dict(confirmed=True, ready=True))
        self.assertEqual(self.lab.command(child, 'native_device_close', **scope, joined=True, risk_refused=False),
            dict(confirmed=True))
        next_scope = self.scope(state, generation)
        self.assertEqual(self.lab.command(child, 'native_device_open', **next_scope), dict(rejected=True))

    def test_unknown_cleanup_retains_native_ownership(self):
        state, child, generation = self.ready()
        scope = self.scope(state, generation)
        self.assertNotIn('rejected', self.lab.command(child, 'native_device_open', **scope))
        self.assertNotIn('rejected', self.lab.command(child, 'native_device_mint', **scope, budget_ms=1000))
        outcome = self.lab.command(child, 'native_device_minted', **scope, sequence=1, joined=False)
        self.assertEqual(outcome, dict(confirmed=False, ready=False))
        status = self.lab.command(child, 'browser_status', account=scope['account'])
        self.assertEqual((status['state'], status['active_commands']), ('unconfirmed', 1))
        next_scope = self.scope(state, generation)
        self.assertEqual(self.lab.command(child, 'native_device_open', **next_scope), dict(rejected=True))

    def test_changed_account_json_refuses_before_restoration(self):
        state = self.lab.state(self._testMethodName)
        with sqlite3.connect(state['root'] / 'framerize.db') as db:
            raw = db.execute('SELECT json FROM accounts').fetchone()[0]
            changed = dict(json.loads(raw), alias='changed-fixture')
            db.execute('UPDATE accounts SET json=?', (json.dumps(changed),))
        child = self.child(state)
        output, _ = child.communicate(timeout=10)
        self.assertNotEqual(child.returncode, 0)
        self.assertEqual(output, b'')
        self.assertIn('saved account state changed', (state['root'] / 'host.stderr').read_text())
        directory = state['root'] / 'cooldown-handoff'
        self.assertFalse(list(directory.glob('claim-*.json')))
        self.assertFalse(list(directory.glob('restored-*.json')))
        self.assertEqual(sha(directory / 'pending.json'), state['pending_hash'])

    def test_completed_handoff_cannot_start_a_second_host(self):
        state, child, _ = self.ready()
        self.lab.stop(child)
        directory = state['root'] / 'cooldown-handoff'
        before = {path.name: sha(path) for path in directory.iterdir()}
        replay = self.child(state, 'replay')
        output, _ = replay.communicate(timeout=10)
        self.assertNotEqual(replay.returncode, 0)
        self.assertEqual(output, b'')
        self.assertIn('FileExistsError', (state['root'] / 'replay.stderr').read_text())
        self.assertEqual({path.name: sha(path) for path in directory.iterdir()}, before)


def main():
    root, package, python = map(Path, sys.argv[1:])
    assert root.is_absolute() and root.is_dir() and not any(root.iterdir())
    assert os.readlink('/proc/self/ns/net') != os.environ['FRAMERIZE_PARENT_NETNS']
    assert not (Path.home() / '.framerize/framerize.db').exists()
    assert not (Path.home() / '.framerize/profiles').exists()
    assert not (Path.home() / '.local/state/framerize-browserless').exists()
    assert not Path('/run/user/' + str(os.getuid()) + '/bus').exists()
    os.umask(0o077)
    lab = None
    result = dict(accepted=False, external_network=False, real_accounts=False,
        scope='Generated host process and synthetic restoration only; no SDK or inference.')
    try:
        lab = Lab(root, package, python)
        RestoredHost.lab = lab
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(RestoredHost)
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
            result['all_owned_processes_joined'] = not lab.children
    group = Path('/sys/fs/cgroup' + Path('/proc/self/cgroup').read_text().strip().split('::', 1)[1])
    result['cgroup'] = str(group)
    result['resources'] = {name: int((group / name).read_text()) for name in
        ('memory.current', 'memory.peak', 'memory.swap.current', 'pids.current', 'pids.peak')}
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
