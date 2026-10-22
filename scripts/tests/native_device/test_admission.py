import asyncio
import json
from collections import deque
from fixture import Fixture, ACCOUNT, LEASE, SECOND


class AdmissionTests(Fixture):
    def test_closed_host_or_inconsistent_absence_cannot_admit(self):
        self.host._closed = True
        with self.assertRaisesRegex(RuntimeError, 'host is closed'):
            self.api.open(self.request)
        self.host._closed = False
        self.life.browser = object()
        with self.assertRaisesRegex(RuntimeError, 'not browserless'):
            self.api.open(self.request)
        self.assertEqual(self.life.active_commands, 0)

    def test_classified_refusal_cools_the_same_device_without_shortening(self):
        self.api.open(self.request)
        self.life.cooling['fixture-primary'] = 1700.0
        self.api.close(dict(self.request, risk_refused=True))
        self.assertEqual(self.life.cooling, {'fixture-primary': 1700.0})
        self.assertEqual(self.life.active_commands, 0)

    def test_unclassified_completion_does_not_invent_a_cooldown(self):
        self.api.open(self.request)
        self.api.close(self.request)
        self.assertEqual(self.life.cooling, {})

    def test_wrong_generation_and_replayed_close_are_rejected(self):
        self.api.open(self.request)
        with self.assertRaisesRegex(RuntimeError, 'stale native host'):
            self.api.close(dict(self.request, generation='b' * 32))
        self.assertEqual(self.life.active_commands, 1)
        self.api.close(self.request)
        with self.assertRaisesRegex(RuntimeError, 'ownership unavailable'):
            self.api.close(self.request)

    def test_symlinked_saved_device_file_is_rejected(self):
        target = self.path.with_name('other.json')
        self.path.rename(target)
        self.path.symlink_to(target)
        with self.assertRaisesRegex(RuntimeError, 'noncanonical device state'):
            self.api.open(self.request)
        self.assertEqual(self.life.active_commands, 0)

    async def test_lost_worker_retains_owner_lock_and_charge(self):
        self.api.open(self.request)
        await self.api.mint(self.request)
        reply = self.api.minted(dict(self.request, joined=False))
        self.assertEqual(reply, dict(confirmed=False, ready=False))
        self.assertEqual(self.life.active_commands, 1)
        self.assertTrue(self.life.native_mint_lock.locked())
        self.assertEqual(list(self.life.requests['fixture-primary']), [1000.0])
        with self.assertRaisesRegex(RuntimeError, 'unconfirmed work'):
            self.api.open(dict(self.request, lease=SECOND))

    async def test_changed_saved_file_refuses_before_mint_charge(self):
        self.api.open(self.request)
        self.path.write_text(json.dumps(['fixture-primary']))
        with self.assertRaisesRegex(RuntimeError, 'saved device changed'):
            await self.api.mint(self.request)
        self.assertFalse(self.life.native_mint_lock.locked())
        self.assertEqual(list(self.life.requests['fixture-primary']), [])
        self.api.close(self.request)

    async def test_scope_cannot_close_before_exact_mint_completion(self):
        self.api.open(self.request)
        await self.api.mint(self.request)
        with self.assertRaisesRegex(RuntimeError, 'mint remains owned'):
            self.api.close(self.request)
        with self.assertRaisesRegex(RuntimeError, 'identity mismatch'):
            self.api.minted(dict(self.request, sequence=2))
        self.assertEqual(self.life.active_commands, 1)
        self.api.minted(self.request)
        self.api.close(self.request)

    async def test_mint_is_paced_and_charged_before_return(self):
        self.life.last_mint = self.now
        self.api.open(self.request)
        reply = await self.api.mint(self.request)
        self.assertEqual(self.delays, [2.5])
        self.assertEqual(reply['sequence'], 1)
        self.assertEqual(list(self.life.requests['fixture-primary']), [1002.5])
        self.assertTrue(self.life.native_mint_lock.locked())
        self.assertEqual(self.api.minted(self.request), dict(confirmed=True, ready=True))
        self.assertFalse(self.life.native_mint_lock.locked())
        self.assertEqual(self.life.last_mint, 1002.5)
        self.api.close(self.request)

    async def test_cancelled_waiter_does_not_release_another_mint(self):
        self.api.open(self.request)
        second = dict(self.request, lease=SECOND)
        self.api.open(second)
        await self.api.mint(self.request)
        task = asyncio.create_task(self.api.mint(second))
        await asyncio.sleep(0)
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertTrue(self.life.native_mint_lock.locked())
        self.assertEqual(len(self.life.requests['fixture-primary']), 1)
        self.api.close(second)
        self.assertEqual(self.life.active_commands, 1)
        self.api.minted(self.request)
        self.api.close(self.request)

    def test_cooldown_is_preserved_and_never_rotates(self):
        self.life.cooling['fixture-primary'] = 1200.0
        with self.assertRaisesRegex(RuntimeError, 'device is cooling'):
            self.api.open(self.request)
        self.assertEqual(self.life.cooling, {'fixture-primary': 1200.0})
        self.assertEqual(self.life.active_commands, 0)

    def test_full_window_refuses_without_a_reserve_device(self):
        self.life.requests['fixture-primary'] = deque([999.0] * 180)
        before = self.path.read_bytes()
        with self.assertRaisesRegex(RuntimeError, 'request window is full'):
            self.api.open(self.request)
        self.assertEqual(self.path.read_bytes(), before)
        self.assertEqual(len(self.life.requests['fixture-primary']), 180)

    def test_expiration_keeps_the_existing_strict_boundary(self):
        self.life.requests['fixture-primary'] = deque([400.0, 400.001])
        self.api.open(self.request)
        self.assertEqual(list(self.life.requests['fixture-primary']), [400.001])
        self.api.close(self.request)

    def test_existing_browser_or_legacy_work_prevents_native_open(self):
        self.life.active_commands = 1
        with self.assertRaisesRegex(RuntimeError, 'other or unconfirmed work'):
            self.api.open(self.request)
        self.life.active_commands, self.life.phase = 0, 'resident'
        with self.assertRaisesRegex(RuntimeError, 'not browserless'):
            self.api.open(self.request)

    def test_restoration_is_required_without_creating_identity(self):
        self.host._cooldown_restoration = {}
        before = self.path.read_bytes()
        with self.assertRaisesRegex(RuntimeError, 'restoration required'):
            self.api.open(self.request)
        self.assertEqual(self.path.read_bytes(), before)
        self.assertEqual(self.life.active_commands, 0)

    def test_primary_device_has_no_fallback(self):
        for primary in (None, 'missing-device'):
            self.life.restored_primary_device = primary
            with self.assertRaisesRegex(RuntimeError, 'primary device unavailable'):
                self.api.open(self.request)
        self.assertEqual(self.life.active_commands, 0)

    def test_open_uses_existing_primary_and_command_ownership(self):
        reply = self.api.open(self.request)
        self.assertEqual(reply, dict(generation=self.host.generation,
                                    lease=LEASE, device='fixture-primary'))
        self.assertEqual(self.life.active_commands, 1)
        self.assertEqual(self.host.accounts, {})
        self.assertTrue(self.api.close(self.request)['confirmed'])
        self.assertEqual(self.life.active_commands, 0)
