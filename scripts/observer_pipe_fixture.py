"""Offline pipe fixture: real protocol/writer, explicitly fake account browser."""
import asyncio
import json
import os
from pathlib import Path
import sys
import uuid

import observed_sidecar as obs
from observer_pipe_writer import AsyncReplyWriter
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'reference/fixtures'))
from observed_sidecar_tests import FakeBrowser, FakeHost

ROOT = Path(os.environ['FRAMERIZE_STATE']).resolve()
assert os.environ['FRAMERIZE_SIDECAR_MODE'] == obs.MODE
assert ROOT.parent.name.startswith('observed-')
assert ROOT.parent.parent.name.startswith('observer-pipe-fixture-')
TRACE = ROOT.parent

def note(event, account='process'):
    assert event.replace('_', '').isalnum()
    assert account == 'process' or str(uuid.UUID(account)) == account
    (TRACE / (event + '.' + account)).write_text('observed\n')

class PipeBrowser(FakeBrowser):
    def __init__(self, account):
        super().__init__()
        self.account = account
    async def send(self, method, calculator, params):
        await super().send(method, calculator, params)
        note('browser_close', self.account)

class PipeHost(FakeHost):
    def __init__(self):
        super().__init__()
        self.owner = None
        self.modes = {}
    async def handle(self, req):
        assert req['cmd'] == 'ai_post', 'fixture handles only explicit AI POST'
        scope = self.owner.scope_for(req)
        body = req['body']
        mode = body['fixture_mode']
        assert mode in ('echo', 'wait', 'die', 'wrong_generation', 'wrong_incarnation')
        self.modes[scope.account] = mode
        if not scope.browsers:
            scope.begin_launch()
            scope.browser_started(PipeBrowser(scope.account))
            self.accounts[scope.account] = 'explicit fake account'
        note('command_started', scope.account)
        if mode == 'die':
            os._exit(73)
        if mode == 'wait':
            while not (TRACE / ('release.' + scope.account)).exists():
                if scope.stopped.is_set():
                    note('stop_seen', scope.account)
                await asyncio.sleep(0.005)
        note('command_finished', scope.account)
        return dict(status=body['status'], body=body['reply'],
                    usage=body['usage'], queue_wait_ms=body['queue_wait_ms'])

class PipeWriter(AsyncReplyWriter):
    def __init__(self, host):
        super().__init__(sys.stdout)
        self.host = host
    async def send(self, frame):
        account = frame.get('account')
        if frame.get('kind') == 'scope_cleanup':
            mode = self.host.modes.get(account)
            if mode in ('wrong_generation', 'wrong_incarnation'):
                frame = dict(frame)
                if mode == 'wrong_generation':
                    frame['application_generation'] += 1
                else:
                    frame['incarnation'] = str(uuid.uuid4())
        await super().send(frame)
        if frame.get('kind') == 'scope_cleanup':
            note('cleanup_sent', account)
        result = frame.get('result')
        if type(result) is dict and result.get('forgotten') is True:
            note('forgotten', result['account'])

def fake_browser_protocol_only():
    # Explicit fixture substitution. No Playwright/browser compatibility claim.
    note('fake_close_protocol')

async def main():
    assert Path(obs.__file__).resolve() == Path(__file__).with_name('observed_sidecar.py').resolve()
    obs.verify_close_protocol = fake_browser_protocol_only
    host = PipeHost()
    owner = obs.ObservedHost(host, ROOT, os.environ['FRAMERIZE_SIDECAR_INCARNATION'],
                             os.environ['FRAMERIZE_OBSERVED_PROFILE_NONCE'])
    host.owner = owner
    (TRACE / 'process.json').write_text(json.dumps(dict(pid=os.getpid(), browser='fake')))
    await owner.loop(PipeWriter(host))

if __name__ == '__main__':
    asyncio.run(main())
