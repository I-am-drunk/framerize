"""Inert negotiation checks against the real native-device authority."""
import sdk_seed_observation as sdk
from fixture import Fixture, LEASE
import test_fresh_account as fresh_fixture


class AuthorityProtocolTests(Fixture):
    prepare = fresh_fixture.FreshAccountTests.prepare
    verify = fresh_fixture.FreshAccountTests.verify
    observe = fresh_fixture.FreshAccountTests.observe
    seed_request = fresh_fixture.FreshAccountTests.seed_request
    fresh = fresh_fixture.FreshAccountTests.fresh
    close_browser = fresh_fixture.FreshAccountTests.close_browser
    history = fresh_fixture.FreshAccountTests.history
    ready = fresh_fixture.FreshAccountTests.ready

    def test_legacy_open_keeps_its_three_field_reply(self):
        reply = self.api.open(self.request)
        self.assertEqual(set(reply), {'generation', 'lease', 'device'})
        self.api.close(self.request)

    async def test_negotiated_observation_requires_authenticated_proof(self):
        await self.ready()
        before = self.history()
        reply = self.api.open(dict(self.request, authority_version=1))
        self.assertEqual(reply['authority'], 'observation')
        self.assertEqual(self.history(), before)
        with self.assertRaisesRegex(RuntimeError, 'observation unavailable'):
            await self.api.mint(self.request)
        grant = self.life.native_grants[LEASE]
        proof = sdk.attach(self.host, self.seed_request(), self.life, grant)
        self.assertIsNotNone(proof)
        request = dict(self.request, **sdk.echo(grant))
        minted = await self.api.mint(request)
        self.assertEqual(minted['observation_id'], proof['observation_id'])
        self.assertEqual(minted['session_id'], proof['session_id'])
        completed = self.api.minted(request)
        self.assertTrue(completed['confirmed'] and completed['ready'])
        self.assertEqual(completed['observation_id'], proof['observation_id'])
        self.api.close(request)

    def test_unknown_authority_version_cannot_acquire_a_grant(self):
        original = self.path.read_bytes()
        for version in (None, False, True, 0, 2, '1'):
            with self.subTest(version=version):
                with self.assertRaisesRegex(RuntimeError, 'unsupported native authority'):
                    self.api.open(dict(self.request, authority_version=version))
                self.assertEqual(self.life.active_commands, 0)
                self.assertFalse(getattr(self.life, 'native_grants', {}))
                self.assertEqual(self.path.read_bytes(), original)

    async def test_negotiated_ledger_keeps_generic_mint_and_completion(self):
        reply = self.api.open(dict(self.request, authority_version=1))
        self.assertEqual(reply['authority'], 'ledger')
        self.assertEqual(len(reply), 4)
        minted = await self.api.mint(self.request)
        self.assertEqual(set(minted), {'generation', 'lease', 'sequence'})
        self.assertEqual(self.api.minted(self.request), {'confirmed': True, 'ready': True})
        self.api.close(self.request)
