"""Fresh, same-host admission without a restored or invented account ledger."""
from collections import deque
from copy import deepcopy

import sdk_seed_observation as sdk
from fixture import Fixture, ACCOUNT, LEASE
from test_sdk_seed_observation import SeedObservationTests


class FreshAccountTests(Fixture):
    prepare = SeedObservationTests.prepare
    verify = SeedObservationTests.verify
    observe = SeedObservationTests.observe
    seed_request = SeedObservationTests.seed_request

    async def test_replaced_identity_or_rate_ledger_cannot_admit(self):
        for field in ('generation', 'incarnation', 'session', 'primary',
                      'cooling', 'requests', 'last_mint'):
            with self.subTest(field=field):
                await self.ready()
                if field == 'generation':
                    self.host.generation = 'b' * 32
                elif field == 'incarnation':
                    self.life.incarnation += 1
                elif field == 'session':
                    sdk.cookie_change(self.host, ACCOUNT, 'changed-session')
                elif field == 'primary':
                    self.life.restored_primary_device = 'fixture-other'
                elif field == 'last_mint':
                    self.life.last_mint = 1.0
                else:
                    setattr(self.life, field, dict(getattr(self.life, field)))
                before = self.history()
                with self.assertRaisesRegex(RuntimeError, 'rate restoration required'):
                    self.api.open(self.request)
                self.assertEqual(self.history(), before)
                self.assertEqual(self.life.native_grants, {})

    def fresh(self):
        if hasattr(self, 'acc'):
            Fixture.setUp(self)
        self.prepare()
        del self.host._cooldown_restoration
        del self.life.restored_primary_device
        self.life.reserve_day, self.life.reserve_used = '2026-10-25', 2

    async def close_browser(self):
        self.life.phase, self.acc.closing = 'closing', True
        return await self.host._finish_close(ACCOUNT, self.life, self.acc)

    def history(self):
        return deepcopy((self.life.cooling, self.life.requests,
                         self.life.last_mint, self.life.reserve_day,
                         self.life.reserve_used)), self.path.read_bytes()

    async def ready(self):
        self.fresh()
        self.verify()
        record = self.observe()
        self.assertTrue(await self.close_browser())
        return record

    async def test_fresh_proof_requires_caller_binding_and_rechecks_before_mint(self):
        await self.ready()
        self.api.open(self.request)
        grant = self.life.native_grants[LEASE]
        for change in ({'user': 'wrong-user'}, {'session_cookie': 'session=wrong'}):
            self.assertIsNone(sdk.attach(self.host, self.seed_request(**change),
                                         self.life, grant))
        before = self.history()
        with self.assertRaisesRegex(RuntimeError, 'observation unavailable'):
            await self.api.mint(self.request)
        request = dict(self.request, **sdk.echo(grant))
        self.life.requests = dict(self.life.requests)
        with self.assertRaisesRegex(RuntimeError, 'observation unavailable'):
            await self.api.mint(request)
        self.assertEqual(self.history(), before)
        self.assertFalse(self.life.native_mint_lock.locked())
        self.api.close(request)

    async def test_absent_browser_and_no_outstanding_work_are_still_required(self):
        for field, value in (('phase', 'unconfirmed'), ('browser', object()),
                             ('close_receipt', object()), ('active_commands', 1)):
            with self.subTest(field=field):
                await self.ready()
                setattr(self.life, field, value)
                with self.assertRaisesRegex(RuntimeError, 'not browserless|other or unconfirmed'):
                    self.api.open(self.request)
                self.assertEqual(self.life.native_grants, {})

    async def test_rate_ledger_replacement_during_spacing_refuses_before_charge(self):
        await self.ready()
        self.api.open(self.request)
        grant = self.life.native_grants[LEASE]
        request = dict(self.request, **sdk.echo(grant))
        self.life.last_mint = self.now
        before = self.history()
        async def replace_ledger(seconds):
            await self.sleep(seconds)
            self.life.cooling = dict(self.life.cooling)
        self.api.sleep = replace_ledger
        with self.assertRaisesRegex(RuntimeError, 'observation changed'):
            await self.api.mint(request)
        self.assertEqual(self.history(), before)
        self.assertFalse(self.life.native_mint_lock.locked())
        self.assertEqual(grant.sequence, 0)
        self.api.close(request)

    async def test_fresh_handoff_preserves_history_and_paces_repeated_mints(self):
        self.fresh()
        self.life.requests['fixture-primary'] = deque([990.0])
        self.life.cooling['fixture-other'] = 2000.0
        self.life.last_mint = self.now
        self.verify()
        record = self.observe()
        before = self.history()
        self.assertTrue(await self.close_browser())
        self.api.open(self.request)
        self.assertEqual(self.history(), before)
        grant = self.life.native_grants[LEASE]
        self.assertIs(grant.sdk_observation, record)
        proof = sdk.attach(self.host, self.seed_request(), self.life, grant)
        request = dict(self.request, **sdk.echo(grant))
        self.assertEqual(proof['observation_id'], record.identity)
        for sequence in (1, 2):
            self.assertEqual((await self.api.mint(request))['sequence'], sequence)
            self.assertTrue(self.api.minted(dict(request, sequence=sequence))['ready'])
        self.assertEqual(self.delays, [2.5, 2.5])
        self.assertEqual(list(self.life.requests['fixture-primary']), [990, 1002.5, 1005])
        self.api.close(request)
        self.assertFalse(hasattr(self.host, '_cooldown_restoration'))
        self.assertFalse(hasattr(self.life, 'native_enrollment_generation'))

    async def test_cookie_only_or_login_without_observation_does_not_admit(self):
        for verified in (False, True):
            with self.subTest(verified=verified):
                self.fresh()
                if verified:
                    self.verify()
                self.assertTrue(await self.close_browser())
                before = self.history()
                with self.assertRaisesRegex(RuntimeError, 'rate restoration required'):
                    self.api.open(self.request)
                self.assertEqual(self.history(), before)
                self.assertEqual(self.life.native_grants, {})

    async def test_actual_cooldown_and_request_capacity_still_refuse(self):
        for reason in ('cooldown', 'capacity'):
            with self.subTest(reason=reason):
                await self.ready()
                if reason == 'cooldown':
                    self.life.cooling['fixture-primary'] = self.now + 5
                else:
                    self.life.requests['fixture-primary'] = deque([self.now] * 180)
                before = self.history()
                with self.assertRaisesRegex(RuntimeError, 'cooling|window is full'):
                    self.api.open(self.request)
                self.assertEqual(self.history(), before)
                self.assertEqual(self.life.native_grants, {})

    async def test_invalid_or_unjoined_browser_cleanup_does_not_admit(self):
        for invalid in ('validation', 'close'):
            with self.subTest(invalid=invalid):
                self.fresh()
                self.verify()
                self.observe()
                if invalid == 'validation':
                    self.validation = {'valid': False}
                else:
                    async def failed_close():
                        raise RuntimeError('inert close refusal')
                    self.receipt.close = failed_close
                await self.close_browser()
                with self.assertRaisesRegex(RuntimeError, 'rate restoration required'):
                    self.api.open(self.request)
                self.assertEqual(self.life.native_grants, {})
