"""Inert coverage for progressive admission fencing and interrupted recovery."""
import json
import unittest
import uuid
import urllib.error
import test_recovery as recovery_fixtures
import test_transition as transition_fixtures

IDS = [str(uuid.UUID(int=value)) for value in (1, 2, 3)]


class PoolDrainTests(unittest.TestCase):
    def fixture(self, kind):
        case = kind()
        case.setUp()
        self.addCleanup(case.doCleanups)
        return case

    def recovery(self):
        case = self.fixture(recovery_fixtures.RecoveryTests)
        case.r.remember('draining')
        return case

    def test_existing_pool_write_prevents_reusing_an_unmarked_plan(self):
        case = self.fixture(transition_fixtures.TransitionTests)
        (case.root / 'pool-write.json').write_text('{}')
        with self.assertRaisesRegex(RuntimeError, 'existing pool write'):
            case.t.preflight(case.plan)
        self.assertEqual(case.posts, [])
        case.command.assert_not_called()

    def drain(self):
        case = self.fixture(transition_fixtures.TransitionTests)
        def http(path, value=None):
            if value is None:
                return case.http(path)
            self.assertEqual(path, '/api/pool/config')
            active = {s['account'] for s in case.slots if s['users']}
            self.assertTrue(active <= set(value['eligible_accounts']))
            record = json.loads((case.root / 'pool-write.json').read_text())
            self.assertEqual(record['before'], case.pool)
            self.assertEqual(record['after'], value)
            case.posts.append(value)
            case.pool = value
            case.slots = [s for s in case.slots if s['account'] in value['eligible_accounts']]
            return value
        case.t.r.http_json.side_effect = http
        return case

    def slot(self, account, users):
        return dict(account=account, users=users, close_started=False)

    def test_idle_accounts_are_fenced_while_active_requests_finish(self):
        case = self.drain()
        case.slots = [self.slot(IDS[0], 1), self.slot(IDS[1], 1), self.slot(IDS[2], 0)]
        def finish_one(_):
            next(s for s in case.slots if s['users'])['users'] = 0
        case.patch(case.t.time, 'sleep', side_effect=finish_one)
        case.t.fence(case.plan, case.saved)
        self.assertEqual([p['eligible_accounts'] for p in case.posts],
                         [IDS[:2], IDS[1:2], []])
        self.assertEqual(case.slots, [])
        case.command.assert_not_called()
        case.t.report.assert_called_with('fenced')

    def test_external_pool_change_prevents_the_next_write(self):
        case = self.drain()
        case.pool = dict(case.saved, active_capacity=3)
        with self.assertRaisesRegex(RuntimeError, 'pool changed during drain'):
            case.t.fence(case.plan, case.saved)
        self.assertEqual(case.posts, [])
        case.command.assert_not_called()

    def test_lost_partial_write_reply_keeps_the_recovery_record(self):
        case = self.drain()
        case.slots = [self.slot(IDS[0], 1), self.slot(IDS[1], 0)]
        update = case.t.r.http_json.side_effect
        def lose_reply(path, value=None):
            result = update(path, value)
            if value is not None:
                raise TimeoutError('synthetic lost reply')
            return result
        case.t.r.http_json.side_effect = lose_reply
        with self.assertRaises(TimeoutError):
            case.t.fence(case.plan, case.saved)
        self.assertEqual(case.pool['eligible_accounts'], IDS[:1])
        record = json.loads((case.root / 'pool-write.json').read_text())
        self.assertEqual(record['after'], case.pool)
        self.assertEqual(record['before'], case.saved)
        case.command.assert_not_called()

    def test_deadline_preserves_busy_accounts_after_partial_fencing(self):
        case = self.drain()
        case.slots = [self.slot(IDS[0], 1), self.slot(IDS[1], 0)]
        def expire(_):
            case.t.time.monotonic.return_value = 1180.0
        case.patch(case.t.time, 'sleep', side_effect=expire)
        with self.assertRaisesRegex(RuntimeError, 'active work did not drain'):
            case.t.fence(case.plan, case.saved)
        self.assertEqual([p['eligible_accounts'] for p in case.posts], [IDS[:1]])
        self.assertEqual(case.slots, [self.slot(IDS[0], 1)])
        case.command.assert_not_called()

    def test_in_progress_closure_cannot_be_replaced_by_a_pool_write(self):
        case = self.drain()
        case.slots = [dict(self.slot(IDS[0], 0), close_started=True)]
        case.patch(case.t.time, 'sleep', side_effect=lambda _: setattr(
            case.t.time.monotonic, 'return_value', 1180.0))
        with self.assertRaisesRegex(RuntimeError, 'active work did not drain'):
            case.t.fence(case.plan, case.saved)
        self.assertEqual(case.posts, [])

    def test_an_account_becoming_busy_cannot_turn_server_refusal_into_success(self):
        case = self.drain()
        case.slots = [self.slot(IDS[0], 1), self.slot(IDS[1], 0)]
        read = case.t.r.http_json.side_effect
        def refuse(path, value=None):
            if value is not None:
                case.slots[1]['users'] = 1
                raise urllib.error.HTTPError('http://fixture', 503, 'busy', {}, None)
            return read(path)
        case.t.r.http_json.side_effect = refuse
        with self.assertRaises(urllib.error.HTTPError):
            case.t.fence(case.plan, case.saved)
        self.assertEqual(case.pool, case.saved)
        self.assertEqual(case.posts, [])
        self.assertTrue((case.root / 'pool-write.json').is_file())
        case.command.assert_not_called()

    def test_interruption_before_or_after_write_restores_only_the_recorded_pool(self):
        for applied in (False, True):
            with self.subTest(applied=applied):
                case = self.recovery()
                partial = dict(case.saved, eligible_accounts=IDS[:1])
                case.r.record_pool_change(case.plan, case.saved, case.saved, partial)
                case.pool = partial if applied else case.saved
                self.assertEqual(case.r.restore()['status'], 'restored-original')
                self.assertEqual(case.pool, case.saved)
                self.assertEqual(case.commands, [])
                self.assertEqual(case.writes, [case.saved] if applied else [])

    def test_repeated_partial_recovery_does_not_restart_or_rewrite(self):
        case = self.recovery()
        case.pool = dict(case.saved, eligible_accounts=IDS[:1])
        case.r.record_pool_change(case.plan, case.saved, case.saved, case.pool)
        first = case.r.restore()
        case.writes.clear()
        self.assertEqual(case.r.restore(), first)
        self.assertEqual(case.commands + case.writes, [])

    def test_a_different_subset_is_not_owned_by_the_latest_write(self):
        case = self.recovery()
        before = dict(case.saved, eligible_accounts=IDS[:2])
        after = dict(case.saved, eligible_accounts=IDS[1:2])
        case.r.record_pool_change(case.plan, case.saved, before, after)
        case.pool = dict(case.saved, eligible_accounts=IDS[:1])
        with self.assertRaisesRegex(RuntimeError, 'outside this transition'):
            case.r.restore()
        self.assertEqual(case.commands + case.writes, [])

    def test_changed_receipt_owner_or_saved_config_is_rejected(self):
        for field in ('version', 'invocation', 'saved_sha256'):
            case = self.recovery()
            case.pool = dict(case.saved, eligible_accounts=IDS[:1])
            case.r.record_pool_change(case.plan, case.saved, case.saved, case.pool)
            path = case.root / 'pool-write.json'
            record = json.loads(path.read_text())
            record[field] = 'unrelated'
            path.write_text(json.dumps(record))
            with self.assertRaisesRegex(RuntimeError, 'record changed'):
                case.r.restore()
            self.assertEqual(case.commands + case.writes, [])

    def test_drain_records_cannot_widen_admission_or_change_other_settings(self):
        case = self.recovery()
        before = dict(case.saved, eligible_accounts=IDS[:1])
        invalid = [dict(before, active_capacity=3),
            dict(before, eligible_accounts='all'),
            dict(before, eligible_accounts=IDS[:2]),
            dict(before, eligible_accounts=IDS[:1] * 2),
            dict(before, eligible_accounts=['not-an-account']),
            dict(before, eligible_accounts=[{}])]
        for after in invalid:
            with self.subTest(after=after):
                with self.assertRaisesRegex(RuntimeError, 'widens admission'):
                    case.r.record_pool_change(case.plan, case.saved, before, after)
        self.assertFalse((case.root / 'pool-write.json').exists())
        self.assertEqual(case.commands + case.writes, [])


if __name__ == '__main__':
    unittest.main()
