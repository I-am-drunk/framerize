"""Synthetic launch-route transfer cases; execution belongs to the runtime owner.

Requires producer 834eee3a over source base 3a608c71. These cases exercise
retained launch inputs, not browser launch, proxy connectivity or SDK execution.
"""
import os
from unittest.mock import patch

import fixture
import sdk_seed_observation as sdk
import test_sdk_seed_observation as seed_fixture

ROUTE_A = 'http://route-a.invalid:8080'
ROUTE_B = 'http://route-b.invalid:8080'


class AmbientRouteTests(fixture.Fixture):
    # Bind helpers only, so discovery does not repeat the producer's test cases.
    prepare = seed_fixture.SeedObservationTests.prepare
    verify = seed_fixture.SeedObservationTests.verify
    observe = seed_fixture.SeedObservationTests.observe
    transfer = seed_fixture.SeedObservationTests.transfer
    seed_request = seed_fixture.SeedObservationTests.seed_request

    def prepare_route(self, registered, captured):
        self.prepare()
        if registered is not None:
            self.host.proxies[fixture.ACCOUNT] = registered
            self.host.proxy_revisions[fixture.ACCOUNT] = 1
        launch = (self.life.incarnation, captured)
        self.life._sdk_launch_route = launch
        self.assertIs(self.life._sdk_launch_browser, self.acc.browser)
        self.verify()
        return launch

    def assert_browser_closed(self, launch):
        self.assertIs(getattr(self.life, '_sdk_launch_route', None), launch)
        self.assertIsNone(self.life._sdk_launch_browser)
        self.assertIsNone(self.life.browser)
        self.assertIsNone(self.life.close_receipt)
        self.assertNotIn(fixture.ACCOUNT, self.host.accounts)
        self.assertEqual(self.life.phase, 'absent')
        self.assertIsNone(self.life.resident_uncertain)
        self.assertFalse(self.life.activity.activities)

    def close_unminted_grant(self):
        grant = self.life.native_grants[fixture.LEASE]
        self.assertEqual(grant.sequence, 0)
        self.assertFalse(self.life.native_mint_lock.locked())
        self.assertFalse(self.life.requests['fixture-primary'])
        self.assertEqual(self.life.cooling, {})
        self.assertEqual(self.delays, [])
        self.assertEqual(self.api.close(self.request), {'confirmed': True})
        self.assertEqual(self.life.active_commands, 0)
        self.assertEqual(self.life.native_grants, {})

    async def test_mismatched_launch_route_refuses_observation_and_transfer(self):
        cases = (('ambient-only', None, ROUTE_A),
                 ('different-proxy', ROUTE_B, ROUTE_A),
                 ('direct-browser', ROUTE_A, None))
        for label, registered, captured in cases:
            with self.subTest(route=label):
                launch = self.prepare_route(registered, captured)
                self.assertIsNone(sdk.begin_observation(self.host, fixture.ACCOUNT, self.page))
                self.assertIsNone(self.life._sdk_session.observation)
                self.assertIsNone(await self.transfer())
                self.assert_browser_closed(launch)
                self.assertEqual(self.events, ['close'])
                self.assertIsNone(self.life.native_grants[fixture.LEASE].sdk_observation)
                self.close_unminted_grant()

    async def test_matching_launch_route_transfers_only_after_joined_close(self):
        for route in (None, ROUTE_A):
            with self.subTest(direct=route is None):
                launch = self.prepare_route(route, route)
                record = self.observe()
                self.assertIs(record.launch_route, launch)
                self.assertTrue(record.live())
                transferred = []
                original = sdk.transfer_closed

                def checked_transfer(host, account, life, acc):
                    self.assertIs(life, self.life)
                    self.assertEqual(self.events, ['validate', 'close'])
                    self.assert_browser_closed(launch)
                    transferred.append(record)
                    return original(host, account, life, acc)

                with patch.object(sdk, 'transfer_closed', checked_transfer):
                    proof = await self.transfer()
                self.assertEqual(transferred, [record])
                self.assertEqual(proof, dict(account=fixture.ACCOUNT,
                    generation=self.host.generation, lease=fixture.LEASE,
                    account_incarnation=self.life.incarnation, user=seed_fixture.USER,
                    session_id=record.session.identity, observation_id=record.identity,
                    seed=seed_fixture.SEED))
                grant = self.life.native_grants[fixture.LEASE]
                self.assertIs(grant.sdk_observation, record)
                self.assertTrue(sdk.ready(self.host, fixture.ACCOUNT, self.life, grant))
                self.assertTrue(record.transferred)
                for name in ('acc', 'owner', 'page', 'context', 'browser', 'frame'):
                    self.assertIsNone(getattr(record, name))
                self.assertEqual(record.session.contexts, [])
                self.close_unminted_grant()

    async def test_later_ambient_changes_cannot_relabel_captured_launch_route(self):
        cases = (('direct', None, None), ('registered', ROUTE_A, ROUTE_A),
                 ('ambient-only', None, ROUTE_A))
        for label, registered, captured in cases:
            for later in (None, ROUTE_B):
                environment = {} if captured is None else {'FRAMERIZE_PROXY': captured}
                # Replace the mapping entirely; never copy or inspect real environment values.
                with self.subTest(route=label, removed=later is None), \
                        patch.object(os, 'environ', environment):
                    launch = self.prepare_route(registered, captured)
                    eligible = registered == captured
                    record = self.observe() if eligible else None
                    if not eligible:
                        self.assertIsNone(sdk.begin_observation(
                            self.host, fixture.ACCOUNT, self.page))
                    environment.clear()
                    if later is not None:
                        environment['FRAMERIZE_PROXY'] = later
                    self.assertIs(self.life._sdk_launch_route, launch)
                    self.assertEqual(launch, (1, captured))
                    self.assertEqual(self.host.proxies.get(fixture.ACCOUNT), registered)
                    if record is not None:
                        self.assertTrue(record.live())
                    else:
                        self.assertIsNone(sdk.begin_observation(
                            self.host, fixture.ACCOUNT, self.page))
                    proof = await self.transfer()
                    self.assert_browser_closed(launch)
                    grant = self.life.native_grants[fixture.LEASE]
                    if eligible:
                        self.assertEqual(proof['observation_id'], record.identity)
                        self.assertEqual(proof['seed'], seed_fixture.SEED)
                        self.assertIs(grant.sdk_observation, record)
                        self.assertTrue(sdk.ready(self.host, fixture.ACCOUNT, self.life, grant))
                        self.assertEqual(self.events, ['validate', 'close'])
                    else:
                        self.assertIsNone(proof)
                        self.assertIsNone(grant.sdk_observation)
                        self.assertEqual(self.events, ['close'])
                    self.close_unminted_grant()

    async def test_unavailable_or_foreign_launch_record_refuses_observation(self):
        cases = (('missing', None), ('none', None), ('list', [1, None]),
                 ('short', (1,)), ('long', (1, None, None)), ('boolean', (True, None)),
                 ('incarnation', (2, None)), ('route-type', (1, 3)),
                 ('browser-missing', (1, None)), ('browser-foreign', (1, None)))
        for label, launch in cases:
            with self.subTest(record=label):
                self.prepare_route(None, None)
                self.life._sdk_launch_route = launch
                if label == 'missing':
                    del self.life._sdk_launch_route
                elif label == 'browser-missing':
                    del self.life._sdk_launch_browser
                elif label == 'browser-foreign':
                    self.life._sdk_launch_browser = object()
                self.assertIsNone(sdk.begin_observation(self.host, fixture.ACCOUNT, self.page))
                self.assertIsNone(self.life._sdk_session.observation)
                self.assertIsNone(await self.transfer())
                self.assert_browser_closed(launch)
                self.assertEqual(self.events, ['close'])
                self.assertIsNone(self.life.native_grants[fixture.LEASE].sdk_observation)
                self.close_unminted_grant()

    async def test_replaced_launch_tuple_refuses_retained_proof_attachment(self):
        for after_close in (False, True):
            with self.subTest(after_close=after_close):
                launch = self.prepare_route(ROUTE_A, ROUTE_A)
                record = self.observe()
                replacement = tuple(list(launch))
                self.assertEqual(replacement, launch)
                self.assertIsNot(replacement, launch)
                if after_close:
                    self.life.phase, self.acc.closing = 'closing', True
                    self.assertTrue(await self.host._finish_close(
                        fixture.ACCOUNT, self.life, self.acc))
                    self.assertTrue(record.transferred)
                    self.assert_browser_closed(launch)
                self.life._sdk_launch_route = replacement
                if after_close:
                    self.api.open(self.request)
                else:
                    self.assertIsNone(await self.transfer())
                reply = await self.host.handle(dict(
                    self.seed_request(), cmd='native_device_sdk_seed'))
                self.assertEqual(reply, {'rejected': True})
                self.assertFalse(record.bound())
                self.assertIsNone(self.life.native_grants[fixture.LEASE].sdk_observation)
                self.assert_browser_closed(replacement)
                self.assertEqual(self.events, ['validate', 'close'] if after_close else ['close'])
                self.close_unminted_grant()
