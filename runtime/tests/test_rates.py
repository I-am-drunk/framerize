"""Rate decisions across restarts, using synthetic clocks and temporary state."""
from copy import deepcopy
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_accounts.journal import RateJournal
from framerize_accounts.rates import AccountRates, RatePolicy


class Clock:
    boot_id = 'boot-one'
    stamp = 1000.0
    today = '2026-10-25'

    def now(self):
        return self.stamp

    def day(self):
        return self.today


class RateTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.journal = RateJournal(directory.name)
        self.addCleanup(self.journal.close)
        self.clock = Clock()
        self.policy = RatePolicy(capacity=2, reserves=1)
        self.rates = AccountRates(self.journal, 'account', self.policy, self.clock)
        self.devices = self.rates.ensure_devices(2)

    def test_request_is_durable_before_caller_can_send_and_is_not_refunded(self):
        device = self.devices[0]
        self.rates.charge_request(device)
        saved = self.journal.load('account')
        self.assertEqual(saved['state']['requests'], [[device, [1000.0]]])
        self.assertFalse(saved['state']['clean'])
        self.rates.charge_request(device)
        with self.assertRaisesRegex(RuntimeError, 'budget'):
            self.rates.charge_request(device)
        self.clock.stamp += 600
        self.rates.charge_request(device)
        self.assertEqual(list(self.rates.state['requests'][device]), [1600.0])

    def test_cooldown_and_primary_survive_close_without_affecting_other_device(self):
        self.rates.cool_device(self.devices[0])
        self.assertEqual(self.rates.select_primary(), self.devices[1])
        self.rates.confirm_runtime_stopped()
        restored = AccountRates(self.journal, 'account', self.policy, self.clock)
        self.assertEqual(restored.ready_devices(), [self.devices[1]])
        self.assertEqual(restored.state['primary'], self.devices[1])
        self.assertEqual(restored.state['cooling'][self.devices[0]], 1600)

    def test_failed_write_changes_no_state_and_closes_further_admission(self):
        before = deepcopy(self.rates.state)
        with patch.object(self.journal, 'save', side_effect=OSError('fixture')):
            with self.assertRaises(OSError):
                self.rates.charge_request(self.devices[0])
        self.assertEqual(self.rates.state, before)
        with self.assertRaisesRegex(RuntimeError, 'recovery'):
            self.rates.charge_request(self.devices[1])
        self.assertEqual(self.journal.load('account')['state']['requests'], [])

    def test_reserve_identity_is_durable_and_clock_jumps_do_not_replenish_it(self):
        reserve = self.rates.allocate_reserve()
        self.clock.today = '2026-10-26'
        self.clock.stamp += 100
        self.assertIsNone(self.rates.allocate_reserve())
        self.clock.stamp += 86400
        self.clock.today = '2026-10-24'
        self.assertIsNone(self.rates.allocate_reserve())
        self.rates.confirm_runtime_stopped()
        restored = AccountRates(self.journal, 'account', self.policy, self.clock)
        self.assertIn(reserve, restored.state['devices'])
        self.assertIn(reserve, restored.state['reserve'])
        self.assertEqual(restored.state['reserve_used'], 1)
        self.clock.today = '2026-10-26'
        self.assertNotEqual(restored.allocate_reserve(), reserve)

    def test_new_boot_preserves_restrictions_without_crediting_unknown_downtime(self):
        self.rates.charge_request(self.devices[0])
        self.rates.cool_device(self.devices[0])
        self.rates.allocate_reserve()
        self.clock.stamp += 50
        self.rates.confirm_runtime_stopped()
        self.clock.boot_id, self.clock.stamp = 'boot-two', 10
        restored = AccountRates(self.journal, 'account', self.policy, self.clock)
        self.assertEqual(restored.state['cooling'][self.devices[0]], 560)
        self.assertEqual(list(restored.state['requests'][self.devices[0]]), [0])
        self.assertEqual(restored.state['reserve_after'], 86410)
        self.assertIsNone(restored.allocate_reserve())

    def test_interrupted_mint_cannot_be_reopened_without_confirmed_cleanup(self):
        self.rates.begin_mint()
        with self.assertRaisesRegex(RuntimeError, 'unfinished mint'):
            self.rates.begin_mint()
        restored = AccountRates(self.journal, 'account', self.policy, self.clock)
        self.assertTrue(restored.needs_cleanup)
        with self.assertRaisesRegex(RuntimeError, 'recovery'):
            restored.charge_request(self.devices[0])
        restored.confirm_runtime_stopped()
        self.assertFalse(restored.state['pending_mint'])
        self.assertFalse(restored.ready_devices())
        self.clock.stamp += 600
        restored.begin_mint()
        self.clock.stamp += 2
        restored.finish_mint()
        self.assertAlmostEqual(restored.mint_ready_in(), 0.25)

    def test_corrupt_or_changed_policy_is_refused_without_rewriting_history(self):
        original = self.journal.load('account')
        for field, value in (('devices', ['repeated', 'repeated']),
                             ('clean', 1), ('last_mint', 1001)):
            altered = deepcopy(original)
            altered['state'][field] = value
            self.journal.save('account', altered)
            with self.subTest(field=field), self.assertRaises(ValueError):
                AccountRates(self.journal, 'account', self.policy, self.clock)
            self.assertEqual(self.journal.load('account'), altered)
        self.journal.save('account', original)
        with self.assertRaisesRegex(ValueError, 'incompatible'):
            AccountRates(self.journal, 'account', RatePolicy(capacity=3), self.clock)

    def test_uncertain_close_preserves_charges_and_does_not_free_other_limits(self):
        device = self.devices[0]
        self.rates.charge_request(device)
        reserve = self.rates.allocate_reserve()
        self.rates.mark_uncertain()
        self.rates.confirm_runtime_stopped()
        self.assertEqual(list(self.rates.state['requests'][device]), [1000])
        self.assertEqual(self.rates.state['reserve_used'], 1)
        self.assertEqual(self.rates.state['reserve'], [reserve])
        self.assertFalse(self.rates.ready_devices())
        other = AccountRates(self.journal, 'other', self.policy, self.clock)
        other_device = other.ensure_devices(1)[0]
        other.charge_request(other_device)
        self.assertEqual(other.ready_devices(), [other_device])

    def test_ambient_request_history_roundtrips_without_aliasing_a_device(self):
        self.rates.charge_request(None)
        self.rates.cool_device(None)
        self.rates.confirm_runtime_stopped()
        restored = AccountRates(self.journal, 'account', self.policy, self.clock)
        self.assertEqual(list(restored.state['requests'][None]), [1000])
        self.assertEqual(restored.ready_devices(), list(self.devices))
        with self.assertRaisesRegex(RuntimeError, 'budget'):
            restored.charge_request(None)
