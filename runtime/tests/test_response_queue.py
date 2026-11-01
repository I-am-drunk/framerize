"""Exercise acknowledged delivery through the actual account RPC owner."""
import asyncio
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_host import app
from framerize_host.browser import driver
from test_lifecycle import FakeBrowser


def chunk(sequence, offset, text, *, done=False, **extra):
    units = len(text.encode('utf-16-le')) // 2
    return dict(kind='chunk', sequence=sequence, offset=offset, next=offset + units,
                body=text, status=200, usage='synthetic', done=done,
                peak_units=units, **extra)


class ResponseQueueTests(unittest.IsolatedAsyncioTestCase):
    async def test_host_preserves_hidden_http_error_and_removes_observers(self):
        listeners = {}
        self.page.on = lambda name, fn: listeners.setdefault(name, []).append(fn)
        self.page.remove_listener = lambda name, fn: listeners[name].remove(fn)
        failure = dict(kind='chunk', sequence=0, offset=0, next=0, done=True,
            status=-1, usage=None, body='TypeError: NetworkError',
            failure_phase='fetch', head_status=None)
        self.install([failure, {'kind': 'closed', 'sequence': 0}])
        original = self.page.evaluate
        async def evaluate(script, args):
            if script == app.AI_START_JS:
                request = SimpleNamespace(url=app.API + '/fixture', method='POST',
                    post_data_json={'turnId': 'fixture-turn'})
                for callback in listeners['response']:
                    callback(SimpleNamespace(request=request, status=504))
            return await original(script, args)
        self.page.evaluate = evaluate
        result = await self.host._run_job(self.page, '/fixture',
            {'turnId': 'fixture-turn'}, 'synthetic-token', 'synthetic-bearer',
            'job', None, 1)
        self.assertEqual(result['status'], -1)
        self.assertIn('HTTP 504 Gateway Timeout', result['body'])
        self.assertEqual(result['network_status'], 504)
        self.assertTrue(all(not values for values in listeners.values()))

    async def asyncSetUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        overrides = patch.multiple(app, ROOT=directory.name, emit=lambda *a, **k: None)
        overrides.start()
        self.addCleanup(overrides.stop)
        protocol = patch.object(driver, 'verify_close_protocol', lambda: None)
        protocol.start()
        self.addCleanup(protocol.stop)
        self.host, self.life, self.browser = app.Host(), app.AccountLifecycle(), FakeBrowser()
        self.life.browser, self.life.phase = self.browser, 'resident'
        self.life.close_receipt = app.BrowserCloseReceipt(self.browser)
        self.page = SimpleNamespace()
        account = app.Account(None, None, self.page, self.life)
        account.browser = self.browser
        self.host.accounts['a'], self.host.lifecycles['a'] = account, self.life
        self.release, self.entered = asyncio.Event(), asyncio.Event()
        self.calls = []

    async def asyncTearDown(self):
        self.release.set()
        await asyncio.gather(*self.life.activity.pending, return_exceptions=True)
        await self.host.close()

    async def run_job(self, timeout=1):
        return await self.host._run_job(self.page, '/fixture', {}, 'synthetic-token',
                                       'synthetic-bearer', 'job', None, timeout)

    def install(self, replies):
        replies = iter(replies)
        async def evaluate(script, args):
            self.calls.append((script, args))
            if script == app.AI_START_JS:
                return 'job'
            if script == app.AI_CANCEL_JS:
                return True
            self.assertEqual(script, app.AI_POLL_JS)
            value = next(replies)
            if isinstance(value, Exception):
                raise value
            return value
        self.page.evaluate = evaluate

    def acknowledgements(self):
        return [args[1] for script, args in self.calls if script == app.AI_POLL_JS]

    async def test_lost_replies_retry_the_same_acknowledgement_without_another_start(self):
        self.install([asyncio.TimeoutError(), chunk(0, 0, 'A'),
                      asyncio.TimeoutError(), chunk(1, 1, 'B', done=True),
                      asyncio.TimeoutError(), {'kind': 'missing'}])
        self.assertEqual(await self.run_job(),
                         dict(status=200, usage='synthetic', body='AB'))
        self.assertEqual(self.acknowledgements(), [-1, -1, 0, 0, 1, 1])
        self.assertEqual(sum(s == app.AI_START_JS for s, _ in self.calls), 1)
        self.assertEqual(sum(s == app.AI_CANCEL_JS for s, _ in self.calls), 1)
        self.assertFalse(self.life.activity.pending)

    async def test_missing_job_before_a_terminal_frame_fails_and_cancels(self):
        self.install([{'kind': 'missing'}])
        with self.assertRaisesRegex(RuntimeError, 'disappeared before completion'):
            await self.run_job()
        self.assertEqual(self.acknowledgements(), [-1])
        self.assertEqual(self.calls[-1][0], app.AI_CANCEL_JS)

    async def test_terminal_failure_discards_prefix_and_private_protocol_fields(self):
        failure = dict(kind='chunk', sequence=1, offset=3, next=3, done=True,
                       status=-1, usage=None, body='synthetic body failure',
                       failure_phase='body', head_status=200, private_extra='not exported')
        self.install([chunk(0, 0, 'old'), failure, {'kind': 'closed', 'sequence': 1}])
        self.assertEqual(await self.run_job(), dict(status=-1, usage=None,
            body='synthetic body failure', failure_phase='body', head_status=200))
        self.assertEqual(self.acknowledgements(), [-1, 0, 1])

    async def test_invalid_successor_is_not_acknowledged(self):
        for change in ({'sequence': 0}, {'offset': 0}, {'next': True},
                       {'status': 503}, {'usage': 'other'}, {'peak_units': 131073}):
            with self.subTest(change=change):
                self.calls.clear()
                bad = dict(chunk(1, 1, 'B', done=True), **change)
                self.install([chunk(0, 0, 'A'), bad])
                with self.assertRaises(RuntimeError):
                    await self.run_job()
                self.assertEqual(self.acknowledgements(), [-1, 0])
                self.assertEqual(self.calls[-1][0], app.AI_CANCEL_JS)

    async def test_deadline_retains_the_underlying_poll_owner(self):
        async def evaluate(script, args):
            if script == app.AI_POLL_JS:
                self.entered.set()
                await self.release.wait()
                return {'kind': 'pending'}
            return 'job' if script == app.AI_START_JS else True
        self.page.evaluate = evaluate
        result = await asyncio.wait_for(self.run_job(0.05), 1)
        self.assertTrue(self.entered.is_set())
        self.assertEqual(result['status'], -1)
        self.assertIn('poll timeout', result['body'])
        self.assertEqual(len(self.life.activity.pending), 1)
        self.assertFalse(await self.host._close_account('a'))
        self.assertTrue(self.browser.connected)
        self.release.set()
        await asyncio.gather(*self.life.activity.pending)
        self.assertTrue(await self.host._close_account('a'))

    async def test_cancellation_retains_the_final_acknowledgement_rpc(self):
        async def evaluate(script, args):
            if script != app.AI_POLL_JS:
                return 'job' if script == app.AI_START_JS else True
            if args[1] == -1:
                return chunk(0, 0, 'complete', done=True)
            self.entered.set()
            await self.release.wait()
            return {'kind': 'closed', 'sequence': 0}
        self.page.evaluate = evaluate
        waiter = asyncio.create_task(self.run_job())
        await asyncio.wait_for(self.entered.wait(), 1)
        waiter.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await waiter
        self.assertEqual(len(self.life.activity.pending), 1)
        self.assertFalse(await self.host._close_account('a'))
        self.release.set()
        await asyncio.gather(*self.life.activity.pending)
        self.assertTrue(await self.host._close_account('a'))
