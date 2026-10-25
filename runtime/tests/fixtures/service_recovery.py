"""Run crash, then recover, in successive invocations of one disposable user unit."""
import json
import os
from pathlib import Path
import subprocess
import sys

RUNTIME = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(RUNTIME))
from framerize_host.accounts.journal import RateJournal
from framerize_host.accounts.rates import AccountRates


def worker(mode, root):
    with RateJournal(root) as journal:
        if mode != 'refuse':
            assert journal.runtime_owner is not None, 'fresh service ownership unavailable'
        if mode == 'write':
            rates = AccountRates(journal, 'fixture')
            device = rates.ensure_devices(1)[0]
            rates.charge_request(device)
            rates.allocate_reserve()
            rates.begin_mint()
            (root.parent / 'before.json').write_text(json.dumps(journal.load('fixture')))
            subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'],
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            os._exit(0)  # Simulate host loss without any journal or browser cleanup.
        before = json.loads((root.parent / 'before.json').read_text())
        rates = AccountRates(journal, 'fixture')
        if mode == 'refuse':
            assert rates.needs_cleanup, 'surviving child incorrectly allowed recovery'
            assert journal.load('fixture') == before
        else:
            assert not rates.needs_cleanup
            after = journal.load('fixture')
            for key in ('devices', 'requests', 'reserve', 'reserve_used', 'primary'):
                assert after['state'][key] == before['state'][key], key
            assert after['state']['clean'] and not after['state']['pending_mint']
            assert rates.ready_devices() == []


if __name__ == '__main__':
    mode, root = sys.argv[1], Path(sys.argv[2])
    if mode in ('write', 'refuse', 'read'):
        worker(mode, root)
    else:
        modes = ('write', 'refuse') if mode == 'crash' else ('read',)
        for step in modes:
            subprocess.run([sys.executable, '-I', '-B', __file__, step, str(root)],
                           check=True, timeout=15)
        print(json.dumps({'phase': mode, 'result': 'pass'}), flush=True)
