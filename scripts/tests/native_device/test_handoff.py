import asyncio
import ast
from collections import deque
from types import SimpleNamespace
from fixture import Fixture, ACCOUNT, SOURCE, host_node
from native_device import install
from resident_activity import ActivityLedger
from test_ownership import host_type


class HandoffTests(Fixture):
    def prepare_close(self, fail=False):
        Host = host_type()
        node = next(n for n in host_node.body
                    if isinstance(n, ast.FunctionDef) and n.name == '_retain_resident_fence')
        namespace = {}
        exec(compile(ast.Module(body=[node], type_ignores=[]), str(SOURCE), 'exec'), namespace)
        Host._retain_resident_fence = namespace['_retain_resident_fence']
        install(Host, self.root, gap=2.5, window=600, capacity=180,
                cooldown=600, clock=lambda: self.now, sleep=self.sleep)
        host = Host()
        host.__dict__.update(vars(self.host))
        browser = object()
        acc = SimpleNamespace(primary_dev='fixture-primary', browser=browser)
        host.accounts[ACCOUNT] = acc
        self.life.phase, self.life.incarnation = 'closing', 1
        self.life.browser = browser
        self.life.close_receipt = SimpleNamespace(browser=browser)
        self.life.activity, self.life.resident_uncertain = ActivityLedger(), None
        del self.life.restored_primary_device
        started, finish = asyncio.Event(), asyncio.Event()
        async def dispose(current, account):
            self.assertIs(current, acc)
            self.assertEqual(account, ACCOUNT)
            started.set()
            await finish.wait()
            if fail:
                raise RuntimeError('fixture cleanup failure')
            current.primary_dev = 'fixture-other'
        host._dispose_account = dispose
        return host, acc, started, finish

    async def test_confirmed_browser_close_hands_actual_device_to_native(self):
        host, acc, started, finish = self.prepare_close()
        history = self.life.requests = {'fixture-other': deque([996.0])}
        cooling = self.life.cooling = {'fixture-other': 999.0}
        closing = asyncio.create_task(host._finish_close(ACCOUNT, self.life, acc))
        await started.wait()
        self.assertEqual(await host.handle(dict(self.request, cmd='native_device_open')),
                         {'rejected': True})
        self.assertFalse(hasattr(self.life, 'restored_primary_device'))
        finish.set()
        self.assertTrue(await closing)
        self.assertIsNone(self.life.resident_uncertain)
        self.assertFalse(self.life.activity.activities)
        opened = await host.handle(dict(self.request, cmd='native_device_open'))
        self.assertEqual(opened['device'], 'fixture-other')
        self.assertIs(self.life.requests, history)
        self.assertIs(self.life.cooling, cooling)
        self.assertEqual(list(history['fixture-other']), [996.0])
        self.assertEqual(self.life.last_mint, 900.0)
        self.assertEqual(await host.handle(dict(self.request, cmd='native_device_close')),
                         {'confirmed': True})

    async def test_unconfirmed_browser_close_cannot_authorize_native(self):
        host, acc, started, finish = self.prepare_close(fail=True)
        closing = asyncio.create_task(host._finish_close(ACCOUNT, self.life, acc))
        await started.wait()
        finish.set()
        self.assertFalse(await closing)
        self.assertEqual(self.life.phase, 'unconfirmed')
        self.assertEqual(self.life.activity.activities, {self.life.resident_uncertain})
        self.assertFalse(hasattr(self.life, 'restored_primary_device'))
        self.assertEqual(await host.handle(dict(self.request, cmd='native_device_open')),
                         {'rejected': True})
        self.assertEqual(self.life.active_commands, 0)
