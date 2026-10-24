"""Inert producer/transfer cases. Execution belongs to the runtime owner."""
import asyncio
import ast
import json
import subprocess
from unittest import TestCase
from unittest.mock import patch
from types import SimpleNamespace

import sdk_seed_observation as sdk
from fixture import Fixture, ACCOUNT, LEASE, SOURCE, host_node
from native_device import install
from resident_activity import ActivityLedger
from test_ownership import host_type

SEED = '00000000000040008000000000000001'
COOKIE = 'session=fixture-sdk-session'
USER = 'fixture-sdk-user'


def host_method(name):
    node = next(n for n in host_node.body
                if isinstance(n, ast.FunctionDef) and n.name == name)
    namespace = {}
    exec(compile(ast.Module(body=[node], type_ignores=[]), str(SOURCE), 'exec'), namespace)
    return namespace[name]


def register_proxy(host, revision, proxy):
    return host_method('_register_proxy')(host, dict(account=ACCOUNT,
        expected_revision=revision - 1, revision=revision, proxy=proxy))


class Emitter:
    def __init__(self):
        self.listeners = {}

    def on(self, name, callback):
        self.listeners.setdefault(name, []).append(callback)

    def remove_listener(self, name, callback):
        self.listeners[name].remove(callback)

    def emit(self, name, *arguments):
        for callback in tuple(self.listeners.get(name, ())):
            callback(*arguments)


class SeedObservationTests(Fixture):
    def prepare(self):
        Host = host_type()
        Host._retain_resident_fence = host_method('_retain_resident_fence')
        install(Host, self.root, gap=2.5, window=600, capacity=180,
                cooldown=600, clock=lambda: self.now, sleep=self.sleep)
        host = Host()
        host.__dict__.update(vars(self.host))
        host.__dict__.pop('_native_devices', None)
        host.proxies, host.proxy_revisions = {}, {}
        browser, context, page = Emitter(), Emitter(), Emitter()
        browser.is_connected, page.is_closed = lambda: True, lambda: False
        context.browser, page.context, page.main_frame = browser, context, object()
        context.pages, browser.contexts = [page], [context]
        device = SimpleNamespace(id='fixture-primary', ctx=context, page=page)
        acc = SimpleNamespace(lifecycle=self.life, browser=browser, ctx=context,
            dev_objs={'fixture-primary': device}, primary_dev='fixture-primary',
            session_cookie=('session', 'fixture-sdk-session'), closing=False)
        host.accounts[ACCOUNT] = acc
        self.life.phase, self.life.incarnation, self.life.browser = 'resident', 1, browser
        self.life._sdk_launch_route, self.life._sdk_launch_browser = (1, None), browser
        self.life.active_commands, self.life.native_grants = 0, {}
        self.life.native_mint_lock = asyncio.Lock()
        self.life.activity, self.life.resident_uncertain = ActivityLedger(), None
        self.events, self.validation = [], {'valid': True}
        async def evaluate(source, arguments):
            self.assertEqual(source, sdk.VALIDATE_JS)
            self.assertEqual(arguments['seed'], SEED)
            self.events.append('validate')
            if isinstance(self.validation, BaseException):
                raise self.validation
            return self.validation
        page.evaluate = evaluate
        async def close():
            self.events.append('close')
            page.emit('close')
            context.emit('close')
            browser.emit('disconnected')
        receipt = SimpleNamespace(browser=browser, close=close)
        self.life.close_receipt = receipt
        async def dispose(current, account):
            self.assertEqual(account, ACCOUNT)
            self.assertIs(current, acc)
            await sdk.close_with_observation(host, ACCOUNT, current, receipt,
                                            asyncio.get_running_loop().time() + 10)
        host._dispose_account = dispose
        self.host, self.acc, self.page, self.receipt = host, acc, page, receipt
        self.api.host = host
        return host

    def verify(self):
        epoch = sdk.begin_session_change(self.host, ACCOUNT)
        sdk.record_verified_session(self.host, ACCOUNT, self.acc, epoch, USER, COOKIE)

    def observe(self):
        record = sdk.begin_observation(self.host, ACCOUNT, self.page)
        self.assertIsNotNone(record)
        self.assertEqual(sdk.observed_result(record, {'value': 'original-token',
            'observation': {'observation_id': record.identity, 'seed': SEED}}), 'original-token')
        return record

    async def transfer(self):
        self.life.phase, self.acc.closing = 'closing', True
        self.assertTrue(await self.host._finish_close(ACCOUNT, self.life, self.acc))
        self.api.open(self.request)
        return sdk.attach(self.host, self.seed_request(), self.life, self.life.native_grants[LEASE])

    def seed_request(self, **changes):
        request = dict(account=ACCOUNT, generation=self.host.generation, lease=LEASE,
                       user=USER, session_cookie=COOKIE)
        request.update(changes)
        return request

    async def test_verified_observation_transfers_only_after_joined_close(self):
        self.prepare()
        self.verify()
        record = self.observe()
        self.assertFalse(record.transferred)
        proof = await self.transfer()
        self.assertEqual(self.events, ['validate', 'close'])
        self.assertEqual(set(proof), {'account', 'generation', 'lease', 'account_incarnation',
                                    'user', 'session_id', 'observation_id', 'seed'})
        self.assertEqual(proof['seed'], SEED)
        self.assertEqual(proof['observation_id'], record.identity)
        self.assertNotEqual(proof['seed'], self.life.native_grants[LEASE].device)
        self.assertIsNone(record.page)
        self.assertEqual(self.life._sdk_session.contexts, [])
        self.assertIsNone(self.life.resident_uncertain)
        self.assertFalse(self.life.activity.activities)
        self.api.close(self.request)

    async def test_prelogin_import_only_and_stale_verified_session_refuse(self):
        self.prepare()
        self.assertIsNone(sdk.begin_observation(self.host, ACCOUNT, self.page))
        self.assertIsNone(sdk.cookie_change(self.host, ACCOUNT, 'fixture-sdk-session'))
        self.assertIsNone(sdk.begin_observation(self.host, ACCOUNT, self.page))
        old = sdk.begin_session_change(self.host, ACCOUNT)
        sdk.begin_session_change(self.host, ACCOUNT)
        sdk.record_verified_session(self.host, ACCOUNT, self.acc, old, USER, COOKIE)
        self.assertIsNone(sdk.begin_observation(self.host, ACCOUNT, self.page))
        self.verify()
        sdk.cookie_change(self.host, ACCOUNT, 'replacement-session')
        self.assertIsNone(sdk.begin_observation(self.host, ACCOUNT, self.page))

    async def test_failed_or_malformed_validation_still_closes_without_proof(self):
        for result in ({'valid': False}, {'valid': 1}, {'valid': True, 'extra': True},
                       None, RuntimeError('inert validation failure')):
            with self.subTest(result=type(result).__name__):
                self.prepare()
                self.verify()
                self.observe()
                self.validation = result
                self.assertIsNone(await self.transfer())
                self.assertEqual(self.events, ['validate', 'close'])
                self.api.close(self.request)

    async def test_cancelled_validation_still_joins_browser_close(self):
        self.prepare()
        self.verify()
        record = self.observe()
        self.validation = asyncio.CancelledError()
        self.life.phase, self.acc.closing = 'closing', True
        with self.assertRaises(asyncio.CancelledError):
            await self.host._finish_close(ACCOUNT, self.life, self.acc)
        self.assertEqual(self.events, ['validate', 'close'])
        self.assertFalse(record.transferred)
        self.assertIsNone(record.seed)
        self.assertEqual(self.life.phase, 'unconfirmed')
        self.assertIn(self.life.resident_uncertain, self.life.activity.activities)

    async def test_failed_close_never_exports_validated_observation(self):
        self.prepare()
        self.verify()
        record = self.observe()
        async def fail_close():
            self.events.append('close-failed')
            raise RuntimeError('inert close failure')
        self.receipt.close = fail_close
        self.life.phase, self.acc.closing = 'closing', True
        self.assertFalse(await self.host._finish_close(ACCOUNT, self.life, self.acc))
        self.assertEqual(self.events, ['validate', 'close-failed'])
        self.assertFalse(record.transferred)
        self.assertIsNone(record.seed)
        self.assertIn(self.life.resident_uncertain, self.life.activity.activities)

    async def test_session_replacement_during_close_refuses_transfer(self):
        self.prepare()
        self.verify()
        record = self.observe()
        original = self.receipt.close
        async def replace():
            sdk.begin_session_change(self.host, ACCOUNT)
            await original()
        self.receipt.close = replace
        self.assertIsNone(await self.transfer())
        self.assertFalse(record.transferred)
        self.api.close(self.request)

    async def test_unsettled_validation_cannot_delay_starting_close(self):
        self.prepare()
        self.verify()
        record = self.observe()
        started, closed, joined = asyncio.Event(), asyncio.Event(), asyncio.Event()
        async def blocked(*_):
            started.set()
            try:
                await asyncio.Event().wait()
            except asyncio.CancelledError:
                await closed.wait()
            joined.set()
        async def no_budget(tasks, **_):
            await started.wait()
            return set(), set(tasks)
        original = self.receipt.close
        async def close():
            await original()
            closed.set()
        self.page.evaluate, self.receipt.close = blocked, close
        with patch.object(sdk.asyncio, 'wait', no_budget):
            self.assertIsNone(await self.transfer())
        self.assertTrue(closed.is_set())
        self.assertTrue(joined.is_set())
        self.assertFalse(record.transferred)
        self.api.close(self.request)

    async def test_wrong_session_user_and_unknown_keys_refuse_same_grant(self):
        self.prepare()
        self.verify()
        self.observe()
        proof = await self.transfer()
        grant = self.life.native_grants[LEASE]
        for change in ({'user': 'foreign'}, {'session_cookie': 'session=foreign'},
                       {'session_cookie': COOKIE + '; session=duplicate'}, {'unknown': True},
                       {'lease': '00000000-0000-0000-0000-000000000000'}):
            with self.subTest(keys=tuple(change)):
                self.assertIsNone(sdk.attach(self.host, self.seed_request(**change), self.life, grant))
        self.assertEqual(sdk.attach(self.host, self.seed_request(), self.life, grant), proof)
        self.api.close(self.request)

    async def test_sdk_mint_echo_and_stale_joined_finish_preserve_cleanup(self):
        self.prepare()
        self.verify()
        self.observe()
        proof = await self.transfer()
        identities = {key: proof[key] for key in ('session_id', 'observation_id')}
        with self.assertRaises(RuntimeError):
            await self.api.mint(self.request)
        request = dict(self.request, **identities)
        minted = await self.api.mint(request)
        self.assertEqual({key: minted[key] for key in identities}, identities)
        sdk.invalidate(self.host, ACCOUNT)
        self.assertEqual(self.api.minted(request), dict(confirmed=True, ready=False, **identities))
        self.assertFalse(self.life.native_mint_lock.locked())
        self.assertEqual(self.api.close(self.request), {'confirmed': True})

    async def test_changed_observation_during_spacing_refuses_without_charge(self):
        self.prepare()
        self.verify()
        self.observe()
        proof = await self.transfer()
        request = dict(self.request, **{key: proof[key] for key in ('session_id', 'observation_id')})
        async def change(seconds):
            self.now += seconds
            sdk.invalidate_observation(self.host, ACCOUNT)
        self.life.last_mint, self.api.sleep = self.now, change
        with self.assertRaises(RuntimeError):
            await self.api.mint(request)
        self.assertFalse(self.life.native_mint_lock.locked())
        self.assertFalse(self.life.requests['fixture-primary'])
        self.api.close(self.request)

    async def test_host_incarnation_and_route_replacement_refuse_old_proof(self):
        for change in ('host', 'incarnation', 'route'):
            with self.subTest(change=change):
                self.prepare()
                self.verify()
                self.observe()
                await self.transfer()
                grant = self.life.native_grants[LEASE]
                if change == 'host':
                    self.host.generation = 'b' * 32
                elif change == 'incarnation':
                    self.life.incarnation += 1
                else:
                    self.host.proxy_revisions[ACCOUNT] = 1
                self.assertFalse(sdk.ready(self.host, ACCOUNT, self.life, grant))
                self.host.generation = self.request['generation']
                self.api.close(self.request)

    async def test_preopen_route_registration_checks_retained_observation(self):
        original, replacement = 'http://route-a.invalid:8080', 'http://route-b.invalid:8080'
        for change in ('unchanged', 'revision', 'replacement', 'direct', 'roundtrip'):
            with self.subTest(change=change):
                self.prepare()
                self.host.proxies[ACCOUNT], self.host.proxy_revisions[ACCOUNT] = original, 1
                self.life._sdk_launch_route = (self.life.incarnation, original)
                self.verify()
                record = self.observe()
                self.life.phase, self.acc.closing = 'closing', True
                self.assertTrue(await self.host._finish_close(ACCOUNT, self.life, self.acc))
                self.assertTrue(record.transferred)
                self.assertEqual(self.life.active_commands, 0)
                target = {'unchanged': original, 'revision': original,
                          'replacement': replacement, 'direct': None, 'roundtrip': replacement}[change]
                revision = 1 if change == 'unchanged' else 2
                self.assertEqual(register_proxy(self.host, revision, target)['outcome'], 'registered')
                if change == 'roundtrip':
                    self.assertEqual(register_proxy(self.host, 3, original)['outcome'], 'registered')
                self.api.open(self.request)
                reply = await self.host.handle(dict(self.seed_request(), cmd='native_device_sdk_seed'))
                if change == 'unchanged':
                    self.assertEqual(reply['observation_id'], record.identity)
                    self.assertIs(self.life.native_grants[LEASE].sdk_observation, record)
                else:
                    self.assertEqual(reply, {'rejected': True})
                    self.assertIsNone(self.life.native_grants[LEASE].sdk_observation)
                self.assertEqual(self.life.native_grants[LEASE].sequence, 0)
                self.assertFalse(self.life.requests['fixture-primary'])
                self.assertEqual(self.api.close(self.request), {'confirmed': True})
                self.assertEqual(self.life.active_commands, 0)
                self.assertEqual(self.events, ['validate', 'close'])

    async def test_effective_route_change_without_revision_refuses_attachment(self):
        for before_open in (True, False):
            with self.subTest(before_open=before_open):
                self.prepare()
                self.verify()
                record = self.observe()
                self.life.phase, self.acc.closing = 'closing', True
                self.assertTrue(await self.host._finish_close(ACCOUNT, self.life, self.acc))
                self.assertTrue(record.transferred)
                if not before_open:
                    self.api.open(self.request)
                self.host.proxies[ACCOUNT] = 'http://replacement.invalid:8080'
                self.assertEqual(self.host.proxy_revisions.get(ACCOUNT, 0), record.revision)
                if before_open:
                    self.api.open(self.request)
                reply = await self.host.handle(dict(self.seed_request(), cmd='native_device_sdk_seed'))
                self.assertEqual(reply, {'rejected': True})
                self.assertIsNone(self.life.native_grants[LEASE].sdk_observation)
                self.assertFalse(self.life.requests['fixture-primary'])
                self.assertEqual(self.api.close(self.request), {'confirmed': True})
                self.assertEqual(self.life.active_commands, 0)
                self.assertEqual(self.events, ['validate', 'close'])

    async def test_ineligible_pages_preserve_primary_until_eligible_replacement(self):
        for completed in (False, True):
            with self.subTest(completed=completed):
                self.prepare()
                self.verify()
                record = sdk.begin_observation(self.host, ACCOUNT, self.page)
                self.assertIsNotNone(record)
                result = {'value': 'original-token', 'observation': {
                    'observation_id': record.identity, 'seed': SEED}}
                if completed:
                    sdk.observed_result(record, result)
                other = SimpleNamespace(id='fixture-other', page=object())
                self.acc.dev_objs['fixture-other'] = other
                for page in (other.page, object()):
                    self.assertIsNone(sdk.begin_observation(self.host, ACCOUNT, page))
                    self.assertIs(self.life._sdk_session.observation, record)
                    self.assertTrue(record.live())
                if not completed:
                    sdk.observed_result(record, result)
                self.assertEqual(record.seed, SEED)
                replacement = self.observe()
                self.assertTrue(record.invalid)
                self.assertIsNone(record.seed)
                self.assertIs(self.life._sdk_session.observation, replacement)
                proof = await self.transfer()
                self.assertEqual(proof['observation_id'], replacement.identity)
                self.api.close(self.request)

    async def test_navigation_and_observation_replacement_revoke_old_record(self):
        self.prepare()
        self.verify()
        prior = self.observe()
        current = self.observe()
        self.assertTrue(prior.invalid)
        self.assertIsNone(prior.seed)
        self.page.emit('framenavigated', self.page.main_frame)
        self.assertTrue(current.invalid)
        self.assertIsNone(await self.transfer())
        self.assertEqual(self.events, ['close'])
        self.api.close(self.request)

    async def test_unconfirmed_sdk_cleanup_retains_original_grant_and_lock(self):
        self.prepare()
        self.verify()
        self.observe()
        proof = await self.transfer()
        identities = {key: proof[key] for key in ('session_id', 'observation_id')}
        request = dict(self.request, **identities)
        await self.api.mint(request)
        self.assertEqual(self.api.minted(dict(request, joined=False)),
                         dict(confirmed=False, ready=False, **identities))
        self.assertTrue(self.life.native_grants[LEASE].uncertain)
        self.assertTrue(self.life.native_mint_lock.locked())
        self.assertEqual(self.life.active_commands, 1)

    async def test_expired_close_deadline_skips_validation_but_still_closes(self):
        self.prepare()
        self.verify()
        record = self.observe()
        await sdk.close_with_observation(self.host, ACCOUNT, self.acc, self.receipt,
                                         asyncio.get_running_loop().time())
        self.assertEqual(self.events, ['close'])
        self.assertFalse(record.transferred)
        self.assertIsNone(record.seed)

    async def test_cancelled_close_waiter_revokes_late_joined_observation(self):
        self.prepare()
        self.verify()
        record = self.observe()
        started, release = asyncio.Event(), asyncio.Event()
        async def close():
            started.set()
            await release.wait()
        self.receipt.close = close
        original, owned = self.host._dispose_account, []
        async def dispose(acc, account):
            task = asyncio.create_task(original(acc, account))
            owned.append(task)
            await asyncio.shield(task)
        self.host._dispose_account = dispose
        self.life.phase, self.acc.closing = 'closing', True
        waiting = asyncio.create_task(self.host._finish_close(ACCOUNT, self.life, self.acc))
        await started.wait()
        waiting.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await waiting
        release.set()
        await asyncio.gather(*owned)
        self.assertTrue(record.invalid)
        self.assertIsNone(record.seed)
        self.assertFalse(record.transferred)
        self.assertIn(self.life.resident_uncertain, self.life.activity.activities)


NODE_PRELUDE = r"""
const assert = require('node:assert/strict');
global.window = global;
global.document = {};
global.location = {href: 'https://framer.com/login'};
let seed = '00000000000040008000000000000001', calls = 0;
global.localStorage = {getItem: () => seed};
const token = 'x'.repeat(600);
const m = {a: async () => {calls++; return token;}};
const url = 'https://app.framerstatic.com/chunk-5LKSDTQ5.mjs';
global.__fzCastleM = m;
global.__fzResidentWrapper = Object.freeze({module: m, callable: m.a, url, entry: 'a', realm: window});
const input = {account: 'fixture', generation: 'fixture-host', account_incarnation: '1',
  session_id: 'fixture-session', observation_id: 'fixture-observation'};
"""


class SeedJavascriptTests(TestCase):
    def run_node(self, body, source=None, observer=None, script=None):
        source = source or "(async () => {const t = await observed(m, url, () => m.a()); return t;})()"
        observer = observer or 'async () => () => ({complete:true, v2:1, v3:0, missing:0})'
        script = script or sdk.OBSERVE_JS.replace('__SOURCE__', source).replace('__OBSERVER__', observer)
        code = (NODE_PRELUDE + '\n(async () => {\nconst observe = (' + script + ');\n'
                + 'const validate = (' + sdk.VALIDATE_JS + ');\n' + body
                + '\n})().catch(() => {process.exitCode = 1;});')
        result = subprocess.run(['node', '-'], input=code, text=True, capture_output=True,
                                timeout=10, check=False)
        self.assertEqual(result.returncode, 0, 'inert SDK observation JavaScript assertions failed')

    def test_success_keeps_original_result_and_exactly_one_existing_call(self):
        self.run_node("const out = await observe(input); assert.equal(out.value, token);"
                      "assert.equal(calls, 1); assert.equal(out.observation.seed, seed);"
                      "assert.deepEqual(validate({...input, seed}), {valid:true});")

    def test_changed_javascript_anchors_refuse_even_with_copied_record(self):
        mutations = ["global.document = {}", "global.localStorage = {getItem: () => seed}",
            "global.__fzCastleM = {...m}", "m.a = async () => token",
            "global.__fzResidentWrapper = Object.freeze({...global.__fzResidentWrapper})",
            "global.__fzOrdinaryV2Observation = Object.freeze({...global.__fzOrdinaryV2Observation})",
            "seed = '00000000000040008000000000000002'", "location.href += '#changed'"]
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                self.run_node("await observe(input); const expected = {...input, seed};"
                              + mutation + ";assert.deepEqual(validate(expected), {valid:false});")

    def test_incomplete_or_non_v2_observation_never_exports_seed(self):
        for counts in ({'complete': False, 'v2': 1, 'v3': 0, 'missing': 0},
                       {'complete': True, 'v2': 0, 'v3': 1, 'missing': 0},
                       {'complete': True, 'v2': 2, 'v3': 0, 'missing': 0}):
            with self.subTest(counts=counts):
                observer = 'async () => () => (' + json.dumps(counts) + ')'
                self.run_node("const out = await observe(input); assert.equal(out.value, token);"
                              "assert.equal(calls, 1); assert.equal(out.observation, null);",
                              observer=observer)

    def test_foreign_javascript_identity_is_not_validation(self):
        self.run_node("await observe(input);"
            "for (const key of Object.keys(input)) {"
            "const changed = {...input, seed, [key]: input[key] + '-foreign'};"
            "assert.deepEqual(validate(changed), {valid:false});}"
            "assert.equal(calls, 1);")

    def test_existing_wrapper_failure_is_not_retried_by_observer(self):
        self.run_node("const failure = new Error('inert');"
                      "m.a = async () => {calls++; throw failure;};"
                      "await assert.rejects(() => observe(input), e => e === failure);"
                      "assert.equal(calls, 1);")

    def test_real_warm_and_mint_sources_keep_results_with_synthetic_observer(self):
        from resident_authorization import RESIDENT_SDK_OBSERVER_JS
        selected = [node for node in ast.parse(SOURCE.read_text()).body
                    if isinstance(node, ast.Assign) and any(isinstance(target, ast.Name)
                    and target.id in ('CASTLE_CHUNK', 'WARM_JS', 'MINT_JS') for target in node.targets)]
        namespace = {}
        exec(compile(ast.Module(body=selected, type_ignores=[]), str(SOURCE), 'exec'), namespace)
        for name, expected in (('WARM_JS', 'true'), ('MINT_JS', 'token')):
            with self.subTest(source=name):
                script = sdk.observation_source(namespace[name]).replace(RESIDENT_SDK_OBSERVER_JS,
                    'async () => () => ({complete:true, v2:1, v3:0, missing:0})')
                self.run_node('const out = await observe(input); assert.equal(out.value, '
                              + expected + '); assert.equal(calls, 1);'
                              'assert.deepEqual(validate({...input, seed}), {valid:true});', script=script)
