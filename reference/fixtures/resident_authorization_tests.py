"""UNRUN inert fixtures: fake browser RPCs, real ledger and provenance class.

Python fixtures check ownership and host bindings, not JavaScript execution.
The separate exact-JS fixture exercises RESIDENT_MINT_JS without a browser.
"""
import ast
import asyncio
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
from resident_activity import ActivityLedger, ResidentRefused
from resident_authorization import ResidentControl, RESIDENT_MINT_JS, IDENTITY

ACCOUNT = '11111111-1111-4111-8111-111111111111'
LEASE = '22222222-2222-4222-8222-222222222222'
NEXT_LEASE = '33333333-3333-4333-8333-333333333333'
URL = 'https://framer.com/login'
WRAPPER = 'https://app.framerstatic.com/chunk-5LKSDTQ5.mjs'
HEADER = 'session=synthetic-session; __cuid=synthetic-device'


class Account(SimpleNamespace):
    @property
    def completed_browser_http200(self):
        evidence = self._completed_browser_http200
        if evidence is not None and not evidence.current():
            evidence.invalidate()
        return self._completed_browser_http200


class Page:
    def __init__(self, context):
        self.context, self.url = context, URL
        self.main_frame = object()
        self.entered, self.release = asyncio.Event(), asyncio.Event()
        self.release.set()
        self.calls, self.hook, self.error = [], None, None
        self.result = {'ok': True, 'token': 'synthetic-private-token'}

    def is_closed(self):
        return False

    async def evaluate(self, source, args):
        assert source == RESIDENT_MINT_JS and not source.startswith('mw:')
        assert args == dict(wrapper_url=WRAPPER, wrapper_export='a',
                            page_url=URL, device='synthetic-device')
        self.calls.append((source, args))
        self.entered.set()
        await self.release.wait()
        if self.hook:
            self.hook()
        if self.error:
            raise self.error
        return dict(self.result)


class Context:
    def __init__(self):
        self.calls, self.hook = [], None
        self.rows = [dict(name=name, value=value, domain='.framer.com',
                          path='/', secure=True) for name, value in
                     [('session', 'synthetic-session'), ('__cuid', 'synthetic-device')]]

    async def cookies(self, url):
        assert url == 'https://api.framer.com/ai/v3/openai/responses/'
        self.calls.append(url)
        if self.hook:
            self.hook()
        return [dict(row) for row in self.rows]


class ResidentAuthorizationTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        source = ROOT / 'scripts/camoufox_host.py'
        node = next(n for n in ast.parse(source.read_text()).body
                    if isinstance(n, ast.ClassDef) and n.name == 'ResidentBrowserHttp200')
        namespace = {}
        exec(compile(ast.Module(body=[node], type_ignores=[]), str(source), 'exec'), namespace)
        self.host = SimpleNamespace(accounts={}, lifecycles={}, generation='fixture-host',
            resident_application_generation=7, _closed=False, _observed=None)
        self.context = Context()
        self.page = Page(self.context)
        self.browser = SimpleNamespace(contexts=[self.context], is_connected=lambda: True)
        self.context.browser, self.context.pages = self.browser, [self.page]
        self.life = SimpleNamespace(activity=ActivityLedger(capacity=2), resident_uncertain=None,
            phase='resident', incarnation=1, browser=self.browser, active_commands=0, close_task=None)
        self.acc = Account(lifecycle=self.life, page=self.page, ctx=self.context,
            browser=self.browser, project='project', dev_objs={}, closing=False, active_requests=0,
            session_cookie=('session', 'synthetic-session'),
            resident_launch=(self.browser, 1, None, False), _completed_browser_http200=None)
        self.host.accounts[ACCOUNT], self.host.lifecycles[ACCOUNT] = self.acc, self.life
        self.acc._completed_browser_http200 = namespace['ResidentBrowserHttp200'](
            self.host, ACCOUNT, self.acc, self.page, 'project')
        self.control = ResidentControl(self.host)
        self.req = dict(version=1, account=ACCOUNT, application_generation=7,
            host_generation='fixture-host', account_incarnation=1, lease=LEASE,
            action='acquire', project='project', path='/ai/v3/openai/responses/',
            wrapper_url=WRAPPER, wrapper_export='a',
            cookie_header='session=synthetic-session', budget_ms=45000)

    async def asyncTearDown(self):
        self.page.release.set()
        await asyncio.gather(*tuple(self.control._owners), return_exceptions=True)

    def terminal(self, action='finish', **extra):
        fields = dict(native_state='settled', admission='resume') if action == 'finish' else {}
        return dict({k: self.req[k] for k in IDENTITY}, action=action, **(fields | extra))

    async def test_one_mint_and_secret_free_idempotent_completion(self):
        grant = await self.control.command(dict(self.req, id=12, cmd='resident_authorization_once'))
        self.assertEqual(grant['cookie_header'], HEADER)
        self.assertEqual(grant['token'], 'synthetic-private-token')
        self.assertIsNone(grant['effective_proxy'])
        self.assertEqual((len(self.page.calls), len(self.context.calls)), (1, 2))
        op = self.life.activity.current._resident_operation
        self.assertIsNone(op.owner.result())
        self.assertIsNone(op.payload)
        self.assertIsNone(op.spec)
        receipt = await self.control.command(self.terminal())
        self.assertEqual(set(receipt), IDENTITY | {'state', 'browser_settled'})
        self.assertEqual((receipt['state'], receipt['browser_settled']), ('released', True))
        self.assertIsNone(op.binding)
        self.assertEqual(await self.control.command(self.terminal()), receipt)
        self.assertEqual(set(op.completion.result()), set(receipt))
        self.assertEqual((len(self.page.calls), len(self.context.calls)), (1, 2))
        with self.assertRaises(ResidentRefused):
            await self.control.command(self.req)

    async def test_login_page_and_extra_same_session_cookies_return_actual_jar(self):
        self.context.rows.append(dict(name='extra', value='synthetic-extra',
                                      domain='.framer.com', path='/', secure=True))
        grant = await self.control.command(self.req)
        self.assertEqual(grant['page_url'], 'https://framer.com/login')
        self.assertEqual(grant['cookie_header'], HEADER + '; extra=synthetic-extra')
        self.assertNotEqual(grant['cookie_header'], self.req['cookie_header'])
        self.assertEqual((len(self.page.calls), len(self.context.calls)), (1, 2))
        await self.control.command(self.terminal())

    async def test_input_session_must_match_actual_and_original_session(self):
        with self.assertRaises(ResidentRefused):
            await self.control.command(dict(self.req, cookie_header='session=different'))
        self.assertEqual((len(self.page.calls), len(self.context.calls)), (0, 1))

    async def test_duplicate_and_mismatched_identity_never_replay_or_release(self):
        await self.control.command(self.req)
        lease = self.life.activity.current
        for req in [self.req, dict(self.terminal(), application_generation=8),
                    dict(self.terminal(), account_incarnation=2),
                    dict(self.terminal(), host_generation='other'),
                    dict(self.terminal(), lease=NEXT_LEASE),
                    dict(self.terminal(), account=NEXT_LEASE)]:
            with self.assertRaises(ResidentRefused):
                await self.control.command(req)
            self.assertIs(self.life.activity.current, lease)
        self.assertEqual((len(self.page.calls), len(self.context.calls)), (1, 2))
        await self.control.command(self.terminal())

    async def test_cancelled_acquire_retains_the_actual_held_evaluation(self):
        self.page.release.clear()
        waiter = asyncio.create_task(self.control.command(self.req))
        await self.page.entered.wait()
        lease = self.life.activity.current
        op, raw = lease._resident_operation, next(iter(self.life.activity.pending))
        waiter.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await waiter
        self.assertFalse(raw.done())
        self.assertFalse(op.owner.done())
        self.assertFalse(op.completion.done())
        self.assertTrue(self.life.activity.fenced)
        self.assertIs(self.life.activity.current, lease)
        with self.assertRaises(ResidentRefused):
            await self.control.command(self.req)
        finish = asyncio.create_task(self.control.command(self.terminal()))
        self.page.release.set()
        receipt = await finish
        self.assertEqual((receipt['state'], receipt['browser_settled']), ('fenced', True))
        self.assertFalse(raw.cancelled())
        self.assertIsNone(op.payload)
        self.assertEqual((len(self.page.calls), len(self.context.calls)), (1, 1))

    async def test_budget_never_cancels_held_rpc_or_unlocks_account(self):
        self.page.release.clear()
        with self.assertRaisesRegex(ResidentRefused, '^resident authorization refused$'):
            await self.control.command(dict(self.req, budget_ms=25))
        self.assertTrue(self.page.entered.is_set())
        raw = next(iter(self.life.activity.pending))
        self.assertFalse(raw.done())
        self.assertTrue(self.life.activity.fenced)
        self.page.release.set()
        receipt = await self.control.command(self.terminal('discard', native_state='unknown'))
        self.assertEqual(receipt['state'], 'fenced')
        self.assertEqual((len(self.page.calls), len(self.context.calls)), (1, 1))

    async def test_finish_closes_admission_and_survives_lost_finish_waiter(self):
        self.page.release.clear()
        acquire = asyncio.create_task(self.control.command(self.req))
        await self.page.entered.wait()
        op = self.life.activity.current._resident_operation
        finish = asyncio.create_task(self.control.command(self.terminal()))
        await asyncio.sleep(0)
        self.assertTrue(op.closed)
        completion = op.completion
        self.assertFalse(completion.done())
        finish.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await finish
        self.assertFalse(completion.cancelled())
        self.page.release.set()
        with self.assertRaises(ResidentRefused):
            await acquire
        receipt = await self.control.command(self.terminal())
        self.assertIs(op.completion, completion)
        self.assertEqual((receipt['state'], receipt['browser_settled']), ('fenced', True))
        self.assertEqual((len(self.page.calls), len(self.context.calls)), (1, 1))

    async def test_unknown_native_state_fences_without_extra_browser_work(self):
        await self.control.command(self.req)
        receipt = await self.control.command(self.terminal('discard', native_state='unknown'))
        self.assertEqual((receipt['state'], receipt['browser_settled']), ('fenced', True))
        self.assertEqual(await self.control.command(self.terminal()), receipt)
        with self.assertRaises(ResidentRefused):
            self.life.activity.activity()
        self.assertEqual((len(self.page.calls), len(self.context.calls)), (1, 2))

    async def test_invalid_wire_data_performs_no_work_or_reservation(self):
        invalid = [dict(extra=True), dict(id=True), dict(cmd='eval'), dict(version=True),
            dict(account='bad'), dict(lease='00000000-0000-0000-0000-000000000000'),
            dict(application_generation=True), dict(account_incarnation=0),
            dict(host_generation='\ud800'), dict(action=[]), dict(budget_ms=45001),
            dict(budget_ms=True), dict(cookie_header='session=x\n'), dict(project=[]),
            dict(path='/auth/ai/access-token'), dict(wrapper_url='https://evil.invalid/a.mjs'),
            dict(wrapper_export=[]), dict(wrapper_url=WRAPPER + '?other=1')]
        for changes in invalid:
            with self.subTest(changes=changes), self.assertRaises(ResidentRefused):
                await self.control.command(self.req | changes)
        self.assertFalse(self.life.activity.leases)
        self.assertEqual((len(self.page.calls), len(self.context.calls)), (0, 0))

    async def test_missing_account_and_provenance_never_get_or_create(self):
        del self.host.accounts[ACCOUNT]
        with self.assertRaises(ResidentRefused):
            await self.control.command(self.req)
        self.assertFalse(self.host.accounts)
        self.host.accounts[ACCOUNT] = self.acc
        self.acc._completed_browser_http200 = None
        with self.assertRaises(ResidentRefused):
            await self.control.command(self.req)
        self.assertFalse(self.life.activity.leases)
        self.assertEqual((len(self.page.calls), len(self.context.calls)), (0, 0))

    async def test_ambiguous_device_stops_before_mint(self):
        self.context.rows.append(dict(self.context.rows[1]))
        with self.assertRaises(ResidentRefused):
            await self.control.command(self.req)
        self.assertTrue(self.life.activity.fenced)
        self.assertEqual((len(self.page.calls), len(self.context.calls)), (0, 1))

    async def test_changed_original_session_stops_before_mint(self):
        self.acc.session_cookie = ('session', 'different-original-session')
        with self.assertRaises(ResidentRefused):
            await self.control.command(self.req)
        self.assertEqual((len(self.page.calls), len(self.context.calls)), (0, 1))

    async def test_post_mint_jar_drift_discards_the_token(self):
        self.page.hook = lambda: self.context.rows[1].update(value='changed-device')
        with self.assertRaises(ResidentRefused):
            await self.control.command(self.req)
        op = self.life.activity.current._resident_operation
        receipt = await op.completion
        self.assertEqual((receipt['state'], receipt['browser_settled']), ('fenced', True))
        self.assertIsNone(op.payload)
        self.assertEqual((len(self.page.calls), len(self.context.calls)), (1, 2))

    async def test_live_binding_drift_discards_token_without_followup_rpc(self):
        changes = [lambda: setattr(self.page, 'url', URL + '?changed'),
            lambda: setattr(self.acc, 'project', 'different-project'),
            lambda: setattr(self.acc, 'resident_launch', (self.browser, 1, 'changed', False)),
            lambda: setattr(self.host, 'generation', 'changed-host'),
            lambda: setattr(self.host, 'resident_application_generation', 8),
            lambda: setattr(self.life, 'incarnation', 2),
            lambda: setattr(self.acc, 'page', object()),
            lambda: setattr(self.page, 'context', object())]
        for change in changes:
            self.page.hook = change
            with self.assertRaises(ResidentRefused):
                await self.control.command(self.req)
            op = self.life.activity.current._resident_operation
            self.assertEqual((await op.completion)['state'], 'fenced')
            self.assertIsNone(op.payload)
            self.assertEqual((len(self.page.calls), len(self.context.calls)), (1, 1))
            await self.asyncTearDown()
            await self.asyncSetUp()

    async def test_rpc_error_reports_unknown_browser_settlement_without_private_error(self):
        self.page.error = RuntimeError('synthetic-private-error-never-export')
        with self.assertRaisesRegex(ResidentRefused, '^resident authorization refused$'):
            await self.control.command(self.req)
        op = self.life.activity.current._resident_operation
        receipt = await op.completion
        self.assertEqual((receipt['state'], receipt['browser_settled']), ('fenced', False))
        self.assertEqual(set(receipt), IDENTITY | {'state', 'browser_settled'})
        self.assertEqual((len(self.page.calls), len(self.context.calls)), (1, 1))

    async def test_invalid_token_or_reply_never_reaches_a_grant(self):
        replies = [{'ok': True, 'token': value} for value in
                   ['', None, True, 'é' * 8193, '\ud800', 'bad\nvalue']]
        replies += [{'ok': False, 'code': 'resident_cache'},
                    {'ok': True, 'token': 'synthetic', 'unexpected': 'private'}]
        for result in replies:
            self.page.result = result
            with self.assertRaisesRegex(ResidentRefused, '^resident authorization refused$'):
                await self.control.command(self.req)
            op = self.life.activity.current._resident_operation
            self.assertEqual((await op.completion)['state'], 'fenced')
            self.assertIsNone(op.payload)
            self.assertEqual((len(self.page.calls), len(self.context.calls)), (1, 1))
            await self.asyncTearDown()
            await self.asyncSetUp()

    async def test_non_evicting_capacity_blocks_new_and_old_leases(self):
        self.life.activity.capacity = 1
        await self.control.command(self.req)
        await self.control.command(self.terminal('discard', native_state='not_started'))
        for lease in (LEASE, NEXT_LEASE):
            with self.assertRaises(ResidentRefused):
                await self.control.command(dict(self.req, lease=lease))
        self.assertEqual(list(self.life.activity.leases), [LEASE])
        self.assertEqual((len(self.page.calls), len(self.context.calls)), (1, 2))

    async def test_actual_rpc_cancellation_cannot_claim_browser_settlement(self):
        self.page.release.clear()
        waiter = asyncio.create_task(self.control.command(self.req))
        await self.page.entered.wait()
        next(iter(self.life.activity.pending)).cancel()
        with self.assertRaises(ResidentRefused):
            await waiter
        op = self.life.activity.current._resident_operation
        receipt = await op.completion
        self.assertEqual((receipt['state'], receipt['browser_settled']), ('fenced', False))
        self.assertEqual((len(self.page.calls), len(self.context.calls)), (1, 1))

    async def test_malformed_finish_and_discard_cannot_close_admission(self):
        await self.control.command(self.req)
        op = self.life.activity.current._resident_operation
        requests = [self.terminal(native_state='unknown'), self.terminal(admission=True),
            self.terminal(project='project'), self.terminal('discard', native_state='settled'),
            self.terminal('discard', native_state='not_started', admission='resume')]
        for request in requests:
            with self.assertRaises(ResidentRefused):
                await self.control.command(request)
            self.assertFalse(op.closed)
        await self.control.command(self.terminal())

    async def test_missing_trusted_application_generation_refuses_construction(self):
        del self.host.resident_application_generation
        with self.assertRaises(ResidentRefused):
            ResidentControl(self.host)
        self.assertEqual((len(self.page.calls), len(self.context.calls)), (0, 0))


if __name__ == '__main__':
    unittest.main()
