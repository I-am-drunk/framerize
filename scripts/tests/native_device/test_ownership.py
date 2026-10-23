import asyncio
import copy
import json
from contextlib import asynccontextmanager
from fixture import Fixture, ACCOUNT, LEASE, SECOND, close_browser
from native_device import install


def host_type():
    class Host:
        _finish_close = close_browser

        async def handle(self, request):
            async with self._use_account(request['account']):
                self.events.append('legacy')
            return True

        @asynccontextmanager
        async def _use_account(self, account):
            life = self.lifecycles[account]
            life.active_commands += 1
            try:
                yield None
            finally:
                life.active_commands -= 1

        def _browser_status(self, account):
            life = self.lifecycles[account]
            return dict(state=life.phase, active_commands=life.active_commands)
    return Host


class OwnershipTests(Fixture):
    async def test_two_admitted_owners_cannot_overspend_last_window_slot(self):
        from collections import deque
        self.life.requests['fixture-primary'] = deque([999.0] * 179)
        second = dict(self.request, lease=SECOND)
        self.api.open(self.request)
        self.api.open(second)
        await self.api.mint(self.request)
        self.api.minted(self.request)
        with self.assertRaisesRegex(RuntimeError, 'request window is full'):
            await self.api.mint(second)
        self.assertEqual(len(self.life.requests['fixture-primary']), 180)
        self.api.close(second)
        self.api.close(self.request)

    async def test_one_account_refusal_leaves_sibling_history_unchanged(self):
        sibling = copy.deepcopy(self.life)
        self.host.lifecycles[SECOND] = sibling
        (self.root / SECOND).mkdir()
        (self.root / SECOND / self.path.name).write_bytes(self.path.read_bytes())
        other = dict(self.request, account=SECOND)
        self.api.open(self.request)
        self.api.open(other)
        self.api.close(dict(self.request, risk_refused=True))
        await self.api.mint(other)
        self.api.minted(other)
        self.api.close(other)
        self.assertEqual(self.life.cooling, {'fixture-primary': 1600.0})
        self.assertEqual(sibling.cooling, {})
        self.assertEqual(len(sibling.requests['fixture-primary']), 1)

    async def test_shared_host_excludes_legacy_until_native_cleanup(self):
        Host = host_type()
        install(Host, self.root, gap=2.5, window=600, capacity=180, cooldown=600,
                clock=lambda: self.now, sleep=self.sleep)
        host = Host()
        host.__dict__.update(self.host.__dict__)
        host.events = []
        await host.handle(dict(self.request, cmd='native_device_open'))
        with self.assertRaisesRegex(RuntimeError, 'account has native work'):
            await host.handle(dict(account=ACCOUNT, cmd='legacy'))
        self.assertEqual(host.events, [])
        await host.handle(dict(self.request, cmd='native_device_close'))
        await host.handle(dict(account=ACCOUNT, cmd='legacy'))
        self.assertEqual(host.events, ['legacy'])

    async def test_unconfirmed_native_work_is_visible_in_existing_status(self):
        Host = host_type()
        install(Host, self.root, gap=2.5, window=600, capacity=180, cooldown=600)
        host = Host()
        host.__dict__.update(self.host.__dict__)
        await host.handle(dict(self.request, cmd='native_device_open'))
        await host.handle(dict(self.request, cmd='native_device_close', joined=False))
        self.assertEqual(host._browser_status(ACCOUNT),
                         dict(state='unconfirmed', active_commands=1))

    async def test_cancelled_pacing_wait_keeps_only_its_original_owner(self):
        entered = asyncio.Event()
        async def wait(_):
            entered.set()
            await asyncio.Event().wait()
        self.life.last_mint = self.now
        self.api.sleep = wait
        self.api.open(self.request)
        task = asyncio.create_task(self.api.mint(self.request))
        await asyncio.wait_for(entered.wait(), 1)
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertFalse(self.life.native_mint_lock.locked())
        self.assertEqual(self.life.active_commands, 1)
        self.assertEqual(list(self.life.requests['fixture-primary']), [])
        self.api.close(self.request)

    async def test_file_change_after_join_rejects_token_but_confirms_cleanup(self):
        self.api.open(self.request)
        await self.api.mint(self.request)
        self.path.write_text(json.dumps(['fixture-other']))
        self.assertEqual(self.api.minted(self.request), dict(confirmed=True, ready=False))
        self.assertEqual(len(self.life.requests['fixture-primary']), 1)
        self.api.close(self.request)
