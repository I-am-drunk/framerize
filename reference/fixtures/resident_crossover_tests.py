"""UNRUN source-only v2 fixtures; fake RPCs and policy, no browser or network.

The v1 fixture class is composed as setup support, not inherited as v2 tests.
Run this file and resident_authorization_tests.py separately under root's cap.
"""
import asyncio
from collections import deque
import json
import time
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, patch

import resident_authorization_tests as v1
from resident_activity import ResidentRefused
from resident_authorization import (IDENTITY, BODY_LIMIT, ResidentControl,
    RESIDENT_CAPTURE_JS, RESIDENT_PIN_JS, RESIDENT_PINNED_MINT_JS,
    RESIDENT_FETCH_JS, _validate_baseline, validate_request)

PROVENANCE = '44444444-4444-4444-8444-444444444444'
MINT_GAP_S, DEV_WINDOW_CAP, DEV_WINDOW_S = 0.0, 3, 600.0
SUCCESS = dict(id='fixture-response', status='completed', incomplete_details=None,
    output=[dict(type='message', content=[dict(type='output_text', text='fixture')])])
BODY = dict(projectId='project', request={'stream': False}, turnId='fixture-turn')


def response(value=SUCCESS, **extra):
    body = json.dumps(value, separators=(',', ':'))
    return dict(dict(status=200, content_type='application/json', body=body,
        bytes=len(body.encode()), complete=True, started=1, finished=1,
        settled=True, failure=None, elapsed_ms=1), **extra)


class Host(SimpleNamespace):
    @staticmethod
    def _request_window(acc, dev, now):
        rows = acc.dev_reqs.get(dev, ())
        while rows and now - rows[0] >= DEV_WINDOW_S:
            rows.popleft()
        return rows

    def _note_request(self, acc, dev):
        acc.dev_reqs.setdefault(dev, deque()).append(time.monotonic())

    def _cool_device(self, acc, dev, account):
        assert account == v1.ACCOUNT
        acc.dev_cooling[dev] = time.monotonic() + DEV_WINDOW_S

    def _mint(self, *args):
        raise AssertionError('ordinary mint must never run')

    _token_acquire = _account = _dev_page = _mint


class Pin:
    def __init__(self, page):
        self.page, self.valid, self.disposed = page, True, False

    async def evaluate(self, source, args):
        page = self.page
        kind = {RESIDENT_PIN_JS: 'check', RESIDENT_PINNED_MINT_JS: 'mint',
                RESIDENT_FETCH_JS: 'fetch'}[source]
        await page.step(kind, args)
        if kind == 'check':
            return self.valid
        if kind == 'mint':
            page.tokens.append('synthetic-token-' + str(len(page.tokens) + 1))
            return dict(ok=True, token=page.fixed_token or page.tokens[-1])
        assert args['bearer'] == 'synthetic-bearer'
        assert json.loads(args['body']) == BODY
        assert args['token'] == (page.fixed_token or page.tokens[0])
        return dict(page.raw)

    async def dispose(self):
        await self.page.step('dispose', {})
        self.disposed = True


class Page(v1.Page):
    def __init__(self, context):
        super().__init__(context)
        self.pin = Pin(self)
        self.raw, self.tokens, self.fixed_token = response(), [], None
        self.hold = self.raise_at = None
        self.after = lambda kind: None

    async def step(self, kind, args):
        self.calls.append((kind, dict(args)))
        if self.hold == kind:
            self.entered.set()
            await self.release.wait()
        self.after(kind)
        if self.raise_at == kind:
            raise RuntimeError('synthetic private exception')

    async def evaluate_handle(self, source, args):
        assert source == RESIDENT_CAPTURE_JS
        await self.step('capture', args)
        return self.pin


class ResidentCrossoverTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        await v1.ResidentAuthorizationTests.asyncSetUp(self)
        self.host = Host(**vars(self.host))
        evidence_type = type(self.acc._completed_browser_http200)
        self.page = Page(self.context)
        self.context.pages, self.acc.page = [self.page], self.page
        self.acc._completed_browser_http200 = evidence_type(
            self.host, v1.ACCOUNT, self.acc, self.page, 'project')
        self.acc._completed_browser_http200.provenance = PROVENANCE
        self.life.cooling, self.life.requests = {}, {}
        self.acc.dev_cooling, self.acc.dev_reqs = self.life.cooling, self.life.requests
        self.acc.last_mint, self.acc.mint_lock = 0.0, asyncio.Lock()
        self.control = ResidentControl(self.host)
        self.req.update(version=2, provenance=PROVENANCE)

    async def asyncTearDown(self):
        self.page.release.set()
        if self.life.activity.current is not None:
            op = self.life.activity.current._resident_operation
            op.close('unknown', False)
            await op.completion
        await asyncio.gather(*tuple(self.control._owners), return_exceptions=True)

    def phase(self, action, **extra):
        return dict({k: self.req[k] for k in IDENTITY}, action=action, **extra)

    def baseline(self, **extra):
        return self.phase('baseline', **(dict(bearer='synthetic-bearer', body=BODY) | extra))

    async def acquired(self):
        return await self.control.command(self.req)

    async def passed(self):
        await self.acquired()
        reply = await self.control.command(self.baseline())
        self.assertTrue(reply['accepted'])
        return reply

    async def test_full_sequence_two_distinct_tokens_and_secret_free_finish(self):
        acquire = await self.acquired()
        self.assertNotIn('token', acquire)
        self.assertEqual(self.page.tokens, [])
        self.assertEqual(acquire['provenance'], PROVENANCE)
        lease = self.life.activity.current
        baseline = await self.control.command(self.baseline())
        self.assertTrue(baseline['accepted'] and baseline['json_object'])
        self.assertIsNone(baseline['refusal'])
        self.assertNotIn('token', baseline)
        native = await self.control.command(self.phase('mint_native'))
        self.assertEqual(native['token'], 'synthetic-token-2')
        self.assertEqual(self.page.tokens, ['synthetic-token-1', 'synthetic-token-2'])
        self.assertIs(self.life.activity.current, lease)
        self.assertEqual(len(self.acc.dev_reqs[None]), 2)
        for reply in (acquire, baseline, native):
            self.assertEqual(reply['version'], 2)
            self.assertEqual(reply['cookie_header'], v1.HEADER)
            self.assertEqual(reply['provenance'], PROVENANCE)
            self.assertTrue(reply['browser_settled'])
        terminal = self.phase('finish', native_state='settled', admission='resume')
        receipt = await self.control.command(terminal)
        self.assertEqual(set(receipt), IDENTITY | {'state', 'browser_settled'})
        self.assertEqual(receipt['state'], 'released')
        self.assertEqual(await self.control.command(terminal), receipt)
        self.assertTrue(self.page.pin.disposed)
        op = lease._resident_operation
        self.assertIsNone(op.spec)
        self.assertIsNone(op.context)
        self.assertIsNone(op.baseline_digest)
        self.assertEqual(sum(k == 'fetch' for k, _ in self.page.calls), 1)

    async def test_out_of_order_and_repeated_phases_do_not_touch_sdk_or_fetch(self):
        await self.acquired()
        before = list(self.page.calls)
        with self.assertRaises(ResidentRefused):
            await self.control.command(self.phase('mint_native'))
        with self.assertRaises(ResidentRefused):
            await self.control.command(self.req)
        self.assertEqual(self.page.calls, before)
        await self.control.command(self.baseline())
        before = list(self.page.calls)
        with self.assertRaises(ResidentRefused):
            await self.control.command(self.baseline())
        self.assertEqual(self.page.calls, before)
        await self.control.command(self.phase('mint_native'))
        before = list(self.page.calls)
        with self.assertRaises(ResidentRefused):
            await self.control.command(self.phase('mint_native'))
        self.assertEqual(self.page.calls, before)

    async def test_json_object_without_completed_inference_cannot_enable_native(self):
        await self.acquired()
        self.page.raw = response({'ok': True})
        reply = await self.control.command(self.baseline())
        self.assertTrue(reply['body_complete'] and reply['json_object'])
        self.assertFalse(reply['accepted'])
        before = list(self.page.calls)
        with self.assertRaises(ResidentRefused):
            await self.control.command(self.phase('mint_native'))
        self.assertEqual(self.page.calls, before)
        self.assertEqual(len(self.page.tokens), 1)

    async def test_fetch_failure_prevents_native_mint(self):
        await self.acquired()
        self.page.raw = response({}, status=None, complete=False, failure='transport')
        reply = await self.control.command(self.baseline())
        self.assertFalse(reply['accepted'])
        self.assertTrue(reply['browser_settled'])
        self.assertIsNone(reply['refusal'])
        with self.assertRaises(ResidentRefused):
            await self.control.command(self.phase('mint_native'))
        self.assertEqual(len(self.page.tokens), 1)

    async def test_unknown_fetch_settlement_fences_account(self):
        await self.acquired()
        self.page.raw = response({}, complete=False, settled=False, finished=0,
                                 failure='unconfirmed')
        reply = await self.control.command(self.baseline())
        self.assertFalse(reply['accepted'] or reply['browser_settled'])
        self.assertTrue(self.life.activity.fenced)
        with self.assertRaises(ResidentRefused):
            await self.control.command(self.phase('mint_native'))
        self.assertEqual(len(self.page.tokens), 1)

    async def cancel_phase(self, stage):
        request = self.req
        if stage == 'fetch':
            await self.acquired()
            request = self.baseline()
        elif stage == 'mint':
            await self.passed()
            request = self.phase('mint_native')
        self.page.hold = stage
        self.page.release.clear()
        waiter = asyncio.create_task(self.control.command(request))
        await asyncio.wait_for(self.page.entered.wait(), 1)
        lease = self.life.activity.current
        op = lease._resident_operation
        raw = next(t for t in self.life.activity.pending if not t.done())
        waiter.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await waiter
        self.assertFalse(raw.done() or op.owner.done() or op.completion.done())
        self.assertTrue(self.life.activity.fenced)
        with self.assertRaises(ResidentRefused):
            await self.control.command(self.phase('mint_native'))
        terminal = asyncio.create_task(self.control.command(
            self.phase('discard', native_state='unknown')))
        await asyncio.sleep(0)
        terminal.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await terminal
        self.assertFalse(op.completion.done())
        self.page.release.set()
        receipt = await asyncio.wait_for(op.completion, 1)
        self.assertEqual(receipt['state'], 'fenced')
        self.assertTrue(receipt['browser_settled'])
        self.assertTrue(self.page.pin.disposed)
        self.assertIs(self.life.activity.current, lease)

    async def test_cancelled_capture_retains_and_disposes_late_handle(self):
        await self.cancel_phase('capture')

    async def test_cancelled_baseline_retains_actual_fetch_and_completion(self):
        await self.cancel_phase('fetch')

    async def test_cancelled_native_mint_retains_actual_sdk_owner(self):
        await self.cancel_phase('mint')

    async def test_concurrent_baselines_refuse_before_another_sdk_call(self):
        await self.acquired()
        self.page.hold = 'fetch'
        self.page.release.clear()
        first = asyncio.create_task(self.control.command(self.baseline()))
        await asyncio.wait_for(self.page.entered.wait(), 1)
        before = list(self.page.calls)
        for req in (self.baseline(), self.phase('mint_native')):
            with self.assertRaises(ResidentRefused):
                await self.control.command(req)
        self.assertEqual(self.page.calls, before)
        self.page.release.set()
        self.assertTrue((await first)['accepted'])

    async def test_raw_rpc_error_reports_unsettled_without_exception_details(self):
        await self.acquired()
        self.page.raise_at = 'fetch'
        reply = await self.control.command(self.baseline())
        self.assertIsNone(reply['refusal'])
        self.assertFalse(reply['accepted'] or reply['browser_settled'])
        self.assertNotIn('private', repr(reply))
        self.assertTrue(self.life.activity.fenced)
        self.assertEqual(len(self.page.tokens), 1)

    async def test_cookie_drift_before_baseline_prevents_mint(self):
        await self.acquired()
        self.context.rows.append(dict(name='extra', value='new',
            domain='.framer.com', path='/', secure=True))
        reply = await self.control.command(self.baseline())
        self.assertFalse(reply['accepted'])
        self.assertEqual(self.page.tokens, [])

    async def test_cookie_drift_during_fetch_prevents_native_mint(self):
        await self.acquired()
        def change(kind):
            if kind == 'fetch':
                self.context.rows[1]['value'] = 'changed-device'
        self.page.after = change
        reply = await self.control.command(self.baseline())
        self.assertFalse(reply['accepted'])
        with self.assertRaises(ResidentRefused):
            await self.control.command(self.phase('mint_native'))
        self.assertEqual(len(self.page.tokens), 1)

    async def test_changed_cached_wrapper_after_baseline_prevents_native_mint(self):
        await self.passed()
        self.page.pin.valid = False
        with self.assertRaises(ResidentRefused):
            await self.control.command(self.phase('mint_native'))
        self.assertEqual(len(self.page.tokens), 1)

    async def test_page_drift_during_mint_never_starts_fetch(self):
        await self.acquired()
        self.page.after = lambda kind: setattr(self.page, 'url', v1.URL + '?changed') \
            if kind == 'mint' else None
        reply = await self.control.command(self.baseline())
        self.assertFalse(reply['accepted'])
        self.assertFalse(any(kind == 'fetch' for kind, _ in self.page.calls))

    async def test_provenance_mismatch_refuses_before_reservation(self):
        req = dict(self.req, provenance=v1.NEXT_LEASE)
        with self.assertRaises(ResidentRefused):
            await self.control.command(req)
        self.assertIsNone(self.life.activity.current)
        self.assertEqual(self.page.calls, [])

    async def test_provenance_drift_between_phases_refuses_without_browser_work(self):
        await self.acquired()
        self.acc._completed_browser_http200.provenance = v1.NEXT_LEASE
        before = list(self.page.calls)
        with self.assertRaises(ResidentRefused):
            await self.control.command(self.baseline())
        self.assertEqual(self.page.calls, before)
        self.assertTrue(self.life.activity.fenced)

    async def test_absolute_budget_is_shared_and_expired_mint_never_runs(self):
        await self.acquired()
        op = self.life.activity.current._resident_operation
        deadline = op.deadline
        await self.control.command(self.baseline())
        self.assertEqual(op.deadline, deadline)
        op.deadline = asyncio.get_running_loop().time() - 1
        before = list(self.page.calls)
        with self.assertRaises(ResidentRefused):
            await self.control.command(self.phase('mint_native'))
        self.assertEqual(self.page.calls, before)

    async def test_same_token_from_two_sdk_calls_is_not_returned_to_native(self):
        self.page.fixed_token = 'synthetic-repeated-token'
        await self.passed()
        with self.assertRaises(ResidentRefused):
            await self.control.command(self.phase('mint_native'))
        self.assertEqual(len(self.page.tokens), 2)

    async def test_device_and_ambient_cooldown_both_prevent_baseline_mint(self):
        for key in ('synthetic-device', None):
            self.acc.dev_cooling[key] = time.monotonic() + 60
        await self.acquired()
        reply = await self.control.command(self.baseline())
        self.assertFalse(reply['accepted'])
        self.assertEqual(self.page.tokens, [])
        self.assertEqual(self.acc.dev_reqs, {})

    async def test_existing_request_window_prevents_native_mint(self):
        await self.passed()
        self.acc.dev_reqs[None] = deque([time.monotonic()] * DEV_WINDOW_CAP)
        with self.assertRaises(ResidentRefused):
            await self.control.command(self.phase('mint_native'))
        self.assertEqual(len(self.page.tokens), 1)
        self.assertEqual(len(self.acc.dev_reqs[None]), DEV_WINDOW_CAP)

    async def test_existing_mint_spacing_is_honored_under_same_lock(self):
        await self.acquired()
        self.acc.last_mint = time.monotonic()
        async def sleep_owned(delay):
            self.assertTrue(self.acc.mint_lock.locked())
            self.assertGreater(delay, 0.1)
            self.assertLessEqual(delay, 0.25)
        with patch(__name__ + '.MINT_GAP_S', 0.25), \
                patch('resident_authorization.asyncio.sleep', new=AsyncMock(side_effect=sleep_owned)) as wait:
            self.assertTrue((await self.control.command(self.baseline()))['accepted'])
            wait.assert_awaited_once()

    async def test_fast_403_keeps_device_cooldown_and_never_mints_native(self):
        await self.acquired()
        self.page.raw = response({'error': 'refused'}, status=403)
        reply = await self.control.command(self.baseline())
        self.assertFalse(reply['accepted'])
        self.assertEqual(reply['refusal'], {'policy': 'castle', 'basis': 'elapsed_fallback'})
        self.assertGreater(self.acc.dev_cooling[None], time.monotonic())
        with self.assertRaises(ResidentRefused):
            await self.control.command(self.phase('mint_native'))

    async def test_strict_wire_validation_rejects_invalid_body_and_phase_fields(self):
        invalid = [self.baseline(bearer='x' * 16385), self.baseline(bearer='bad\nvalue'),
            self.baseline(body=[]), self.baseline(body={'castleToken': 'forbidden'}),
            self.baseline(body={'nested': {'castleToken': 'forbidden'}}),
            self.baseline(body={'n': float('nan')}), self.baseline(body={'pad': 'x' * BODY_LIMIT}),
            self.phase('mint_native', bearer='unexpected'),
            dict(self.req, provenance=None), dict(self.req, provenance='not-uuid'),
            {k: value for k, value in self.req.items() if k != 'provenance'},
            dict(self.req, version=1), dict(self.baseline(), version=1)]
        numbers = [2 ** 53, -(2 ** 53), 2 ** 53 + 1, -(2 ** 53 + 1),
            2 ** 64 - 1, -(2 ** 63)]
        numbers.extend(json.loads(spelling) for spelling in (
            '9007199254740992.0', '-9007199254740992.0',
            '9007199254740993.0', '-9007199254740993.0',
            '9.007199254740993e15', '-9.007199254740993e15',
            '9007199254740992.5', '-9007199254740992.5', '1e100', '-1e100'))
        numbers.extend((float('nan'), float('inf'), float('-inf')))
        for number in numbers:
            req = self.baseline(body={'request': {'tools': [
                {'parameters': {'minimum': number}}]}})
            with self.subTest(number=number):
                with self.assertRaises(ResidentRefused):
                    _validate_baseline(req)
            invalid.append(req)
        for req in invalid:
            with self.subTest(action=req['action']):
                with self.assertRaises(ResidentRefused):
                    await self.control.command(req)
        self.assertEqual(self.page.calls, [])
        self.assertIsNone(self.life.activity.current)

    async def test_wire_byte_boundaries_and_body_ownership(self):
        body = {'pad': 'x' * (BODY_LIMIT - len('{"pad":""}'))}
        checked = validate_request(self.baseline(bearer='x' * 16384, body=body))
        body['pad'] = 'changed'
        self.assertEqual(len(checked['body']['pad']), BODY_LIMIT - 10)
        with self.assertRaises(ResidentRefused):
            validate_request(self.baseline(body={'pad': 'é' * 32764}))
        for spelling in ('-9007199254740991', '0', '9007199254740991',
                '-9007199254740991.0', '9007199254740991.0',
                '-9.007199254740991e15', '9.007199254740991e15',
                '0.25', '-0.25', '0.1', '-0.0', 'true', 'false', 'null'):
            value = json.loads(spelling)
            body = {'request': {'tools': [{'parameters': {'minimum': value}}]}}
            req = self.baseline(body=body)
            with self.subTest(spelling=spelling):
                for validate in (_validate_baseline, validate_request):
                    checked = validate(req)
                    actual = checked['body']['request']['tools'][0]['parameters']['minimum']
                    self.assertEqual(actual, value)
                    self.assertIs(type(actual), type(value))
                    self.assertIsNot(checked['body']['request'], body['request'])

    async def test_discard_after_snapshot_releases_without_mint(self):
        await self.acquired()
        req = self.phase('discard', native_state='not_started')
        receipt = await self.control.command(req)
        self.assertEqual(receipt['state'], 'released')
        self.assertEqual(receipt['version'], 2)
        self.assertEqual(set(receipt), IDENTITY | {'state', 'browser_settled'})
        self.assertEqual(await self.control.command(req), receipt)
        self.assertEqual(self.page.tokens, [])

    async def test_absolute_timeout_retains_pending_fetch_until_it_settles(self):
        await self.acquired()
        op = self.life.activity.current._resident_operation
        op.deadline = asyncio.get_running_loop().time() + 0.03
        self.page.hold = 'fetch'
        self.page.release.clear()
        waiter = asyncio.create_task(self.control.command(self.baseline()))
        await asyncio.wait_for(self.page.entered.wait(), 1)
        with self.assertRaises(ResidentRefused):
            await waiter
        self.assertFalse(op.owner.done() or op.completion.done())
        self.assertTrue(self.life.activity.fenced)
        self.page.release.set()
        self.assertEqual((await op.completion)['state'], 'fenced')

    async def test_dispose_waiter_loss_does_not_drop_cleanup_owner(self):
        await self.acquired()
        self.page.hold = 'dispose'
        self.page.release.clear()
        waiter = asyncio.create_task(self.control.command(
            self.phase('discard', native_state='not_started')))
        await asyncio.wait_for(self.page.entered.wait(), 1)
        op = self.life.activity.current._resident_operation
        waiter.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await waiter
        self.assertFalse(op.completion.done())
        self.page.release.set()
        self.assertEqual((await op.completion)['state'], 'released')


    async def test_cancelled_phase_owner_keeps_raw_rpc_before_handle_disposal(self):
        await self.acquired()
        self.page.hold = 'fetch'
        self.page.release.clear()
        waiter = asyncio.create_task(self.control.command(self.baseline()))
        await asyncio.wait_for(self.page.entered.wait(), 1)
        op = self.life.activity.current._resident_operation
        raw = next(t for t in self.life.activity.pending if not t.done())
        op.owner.cancel()
        reply = await waiter
        self.assertFalse(reply['browser_settled'] or reply['accepted'])
        self.assertFalse(raw.done())
        op.close('unknown', False)
        await asyncio.sleep(0)
        self.assertFalse(op.completion.done() or self.page.pin.disposed)
        self.page.release.set()
        receipt = await op.completion
        self.assertEqual(receipt['state'], 'fenced')
        self.assertFalse(receipt['browser_settled'])


class ProtocolOnePreservationTests(unittest.IsolatedAsyncioTestCase):
    asyncSetUp = v1.ResidentAuthorizationTests.asyncSetUp
    asyncTearDown = v1.ResidentAuthorizationTests.asyncTearDown
    terminal = v1.ResidentAuthorizationTests.terminal

    async def test_original_single_mint_acquire_and_receipts_remain_unchanged(self):
        await v1.ResidentAuthorizationTests.test_one_mint_and_secret_free_idempotent_completion(self)


if __name__ == '__main__':
    unittest.main()
