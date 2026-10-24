"""Exercise actual host dispatch with synthetic ports and fixed event fields."""
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import resident_http200_provenance_tests as fixture


class AiPostFailureTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.f = fixture.ResidentHttp200Tests()
        await self.f.asyncSetUp()
        self.charges = []
        self.f.host._note_request = lambda *args: self.charges.append(args)
        self.f.host._cool_device = lambda *args: self.fail('diagnostic caused cooldown')

    async def dispatch(self, result):
        self.f.result = result
        request = self.f.request()
        request.pop('defer_retries')
        returned = await self.f.host._handle(request)
        self.assertIs(returned, result)
        self.assertNotIn('failure_phase', returned)
        self.assertNotIn('head_status', returned)
        self.assertEqual(tuple(map(len, (self.f.mints, self.f.sends,
                                        self.f.releases, self.charges))), (1, 1, 1, 1))
        events = [data for args, data in self.f.events if args == ('ai.post',)]
        self.assertEqual(len(events), 1)
        for secret in ('synthetic-token-secret', 'synthetic-bearer-secret',
                       'private-error', 'private-cookie'):
            self.assertNotIn(secret, repr(self.f.events))
        return events[0]

    async def check_failure(self, phase, head_status):
        result = dict(status=-1, usage=None, body='private-error',
                      failure_phase=phase, head_status=head_status,
                      cookie='private-cookie')
        event = await self.dispatch(result)
        self.assertEqual(event['status'], -1)
        self.assertEqual(event['outcome'], 'error')
        self.assertEqual(event['failure_phase'], phase)
        self.assertEqual(event['head_status'], head_status)
        self.assertRegex(event['job'], r'^job-[0-9a-f]{32}$')
        self.assertEqual(set(event), {'account', 'path', 'status', 'duration_ms',
            'queue_wait_ms', 'outcome', 'failure_phase', 'head_status', 'job'})
        self.assertIsNone(self.f.acc.completed_browser_http200)
        self.assertNotIn('_retry_device', result)

    async def test_fetch_failure_logs_only_fixed_facts(self):
        await self.check_failure('fetch', None)

    async def test_body_failure_after_403_does_not_retry_or_cool(self):
        await self.check_failure('body', 403)

    async def test_body_failure_after_200_grants_no_provenance(self):
        await self.check_failure('body', 200)

    async def test_legacy_poll_timeout_does_not_invent_a_fetch_stage(self):
        event = await self.dispatch(dict(status=-1, usage=None, body='private-error'))
        self.assertNotIn('failure_phase', event)
        self.assertNotIn('head_status', event)

    async def test_completed_response_does_not_log_supplied_failure_fields(self):
        event = await self.dispatch(dict(status=200, usage=None, body='private-error',
                                        failure_phase='body', head_status=403))
        self.assertNotIn('failure_phase', event)
        self.assertNotIn('head_status', event)

    def test_malformed_diagnostic_pairs_are_not_exported(self):
        project = self.f.host._take_ai_post_failure_facts
        invalid = [('fetch', 200), ('body', None), ('private-error', 200),
                   ('body', True), ('body', 200.0), ('body', '200'),
                   ('body', -1), ('body', 99), ('body', 600),
                   (['body'], 200), ('body', {'private-cookie': 200})]
        for phase, status in invalid:
            with self.subTest(phase=phase, status=status):
                result = dict(status=-1, failure_phase=phase, head_status=status)
                self.assertEqual(project(result, 'job-fixture'), {})
                self.assertEqual(result, {'status': -1})
        for status in (-1.0, '-1', None, False, 200):
            self.assertEqual(project(dict(status=status, failure_phase='fetch',
                                          head_status=None), 'job-fixture'), {})


if __name__ == '__main__':
    unittest.main()
