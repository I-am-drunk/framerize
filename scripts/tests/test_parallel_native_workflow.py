import json
import unittest
from types import SimpleNamespace
from unittest.mock import patch
from native_check_fixture import Scenario, quiet_pool
from parallel_native_check import check, client
from parallel_native_check.support import Refusal


def pending_close_pool():
    value = quiet_pool()
    value['residency']['residents'] = [{
        'account': 'account', 'users': 0, 'status': 'closing',
        'replacement': None, 'uncertain': False, 'close_started': False}]
    return value


class WorkflowTests(unittest.TestCase):
    def test_missing_send_snapshot_never_proves_no_retry(self):
        scenario = Scenario()
        self.prepare(scenario)
        original = scenario.events.poll
        def poll():
            original()
            scenario.events.send_snapshots.clear()
        scenario.events.poll = poll
        with patch.object(check.time, 'monotonic', side_effect=range(0, 1000, 100)):
            with self.assertRaisesRegex(Refusal, 'native_check_timeout'):
                check.run(scenario.binding, scenario.events, scenario.report)
        self.assertEqual(scenario.report['post_calls'], 1)

    def test_started_stream_requires_send_invocation(self):
        scenario = Scenario()
        self.prepare(scenario)
        original = scenario.events.poll
        def poll():
            original()
            scenario.events.send_invocations.clear()
        scenario.events.poll = poll
        with self.assertRaisesRegex(Refusal, 'native_send_count'):
            check.run(scenario.binding, scenario.events, scenario.report)
        self.assertEqual(scenario.report['post_calls'], 1)

    def test_extra_send_without_headers_prevents_second_request(self):
        scenario = Scenario()
        self.prepare(scenario)
        original = scenario.events.poll
        def poll():
            original()
            if scenario.events.completed:
                scenario.events.send_invocations[:] = [1, 2]
        scenario.events.poll = poll
        with self.assertRaisesRegex(Refusal, 'unexpected_native_send'):
            check.run(scenario.binding, scenario.events, scenario.report)
        self.assertEqual(scenario.report['post_calls'], 1)

    def test_unlogged_send_before_headers_is_caught_by_cleanup_counter(self):
        scenario = Scenario()
        self.prepare(scenario)
        original = scenario.events.poll
        def poll():
            original()
            if scenario.events.send_snapshots:
                scenario.events.send_snapshots[-1]['send_invocations'] = 2
        scenario.events.poll = poll
        with self.assertRaisesRegex(Refusal, 'native_send_snapshot'):
            check.run(scenario.binding, scenario.events, scenario.report)
        self.assertEqual(scenario.report['post_calls'], 1)

    def test_pending_close_prevents_first_request(self):
        scenario = Scenario()
        self.prepare(scenario)
        idle = pending_close_pool()
        idle['residency']['residents'][0]['status'] = 'idle'
        self.assertTrue(check.quiet(idle, 'account'))
        self.assertTrue(check.quiet(quiet_pool(), 'account'))
        with patch.object(client, 'pool', return_value=pending_close_pool()):
            with self.assertRaisesRegex(Refusal, 'native_work_before_test'):
                check.run(scenario.binding, scenario.events, scenario.report)
        self.assertEqual(scenario.report['post_calls'], 0)

    def test_pending_close_after_cancellation_prevents_second_request(self):
        scenario = Scenario()
        self.prepare(scenario)
        with patch.object(client, 'pool', side_effect=[quiet_pool(), pending_close_pool()]):
            with patch.object(check.time, 'monotonic', side_effect=range(0, 1000, 100)):
                with self.assertRaisesRegex(Refusal, 'native_check_timeout'):
                    check.run(scenario.binding, scenario.events, scenario.report)
        self.assertEqual(scenario.report['post_calls'], 1)

    def prepare(self, scenario):
        self.enterContext(patch.object(client, 'pool', return_value=quiet_pool()))
        self.head = self.enterContext(patch.object(client, 'head', return_value={'status': 200}))
        self.enterContext(patch.object(client, 'send', side_effect=scenario.send))

    def test_two_requests_wait_for_cancel_cleanup_then_complete(self):
        scenario = Scenario()
        self.prepare(scenario)
        check.run(scenario.binding, scenario.events, scenario.report)
        self.assertTrue(scenario.report['accepted'])
        self.assertEqual(scenario.report['post_calls'], 2)
        self.assertEqual(len(scenario.events.completed), 2)
        self.assertTrue(scenario.sent[0].cancelled)
        self.head.assert_called_once()

    def test_uncertain_cleanup_prevents_second_request(self):
        scenario = Scenario(cleanup=False)
        self.prepare(scenario)
        with self.assertRaisesRegex(Refusal, 'native_cleanup_outcome'):
            check.run(scenario.binding, scenario.events, scenario.report)
        self.assertEqual(scenario.report['post_calls'], 1)

    def test_missing_completion_does_not_prove_cleanup(self):
        scenario = Scenario(emit=False)
        self.prepare(scenario)
        with patch.object(check.time, 'monotonic', side_effect=range(0, 1000, 100)):
            with self.assertRaisesRegex(Refusal, 'native_check_timeout'):
                check.run(scenario.binding, scenario.events, scenario.report)
        self.assertEqual(scenario.report['post_calls'], 1)

    def test_admission_refusal_stops_sequence(self):
        scenario = Scenario()
        scenario.events.refusals = 1
        self.prepare(scenario)
        with self.assertRaisesRegex(Refusal, 'admission_refused'):
            check.run(scenario.binding, scenario.events, scenario.report)
        self.assertEqual(scenario.report['post_calls'], 1)
        self.assertTrue(scenario.sent[0].state['finished'])

    def test_failed_sequence_observes_cleanup_without_another_send(self):
        scenario = Scenario()
        self.prepare(scenario)
        scenario.send(scenario.binding, True, scenario.report).close(cancel=True)
        check.observe_failed_cleanup(scenario.binding, scenario.events, scenario.report)
        self.assertTrue(scenario.report['cleanup_after_error']['confirmed'])
        self.assertEqual(scenario.report['post_calls'], 1)

    def test_two_post_limit_is_enforced_before_connecting(self):
        with patch.object(client.http.client, 'HTTPConnection') as connection:
            with self.assertRaisesRegex(Refusal, 'post_limit'):
                client.send(None, False, {'post_calls': 2})
        connection.assert_not_called()

    def test_request_uses_only_isolated_route_and_explicit_account(self):
        binding = SimpleNamespace(account='inert-account', input={'context': {'fixture': True}})
        report = {'post_calls': 0}
        with patch.object(client.http.client, 'HTTPConnection') as connection:
            with patch.object(client, 'Stream'):
                client.send(binding, True, report)
        self.assertEqual(connection.call_args.args, ('127.0.0.1', 8788))
        call = connection.return_value.request.call_args
        self.assertEqual(call.args, ('POST', '/openai/v1/responses'))
        self.assertEqual(call.kwargs['headers']['x-framerize-account'], 'inert-account')
        payload = json.loads(call.kwargs['body'])
        self.assertTrue(payload['stream'])
        self.assertEqual(payload['max_output_tokens'], 512)
        self.assertEqual(report['post_calls'], 1)
