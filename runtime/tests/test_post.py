"""Browser request accounting and safe diagnostics at the real dispatch boundary."""
from pathlib import Path
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_host import app


class PostTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.events = Mock()
        overrides = patch.multiple(app, ROOT=directory.name, emit=self.events)
        overrides.start()
        self.addCleanup(overrides.stop)
        self.host = app.Host()
        self.acc = app.Account(None, None, object())
        self.acc.devices = ['device']
        self.host._account = AsyncMock(return_value=self.acc)
        self.host._ensure_editor = AsyncMock()
        self.host._set_editor = Mock()
        self.host._pick_device_sticky = Mock(return_value='device')
        self.host._token_acquire = AsyncMock(return_value=('slot', 'private-token', 'device'))
        self.host._token_release = Mock()
        self.host._note_request = Mock()
        self.host._cool_device = Mock(side_effect=AssertionError('unexpected cooldown'))

    async def dispatch(self, result, request=None):
        self.host._run_job = AsyncMock(return_value=result)
        returned = await self.host._handle(dict(cmd='ai_post', account='fixture',
            path='/ai/v3/responses/', bearer='private-bearer',
            body={'projectId': 'project', 'request': request or {'model': 'fixture'}}))
        self.assertIs(returned, result)
        self.host._token_acquire.assert_awaited_once()
        self.host._run_job.assert_awaited_once()
        self.host._note_request.assert_called_once_with(self.acc, 'device')
        self.host._token_release.assert_called_once_with(self.acc, 'slot')
        self.assertNotIn('failure_phase', returned)
        self.assertNotIn('head_status', returned)
        event = self.events.call_args.kwargs
        for secret in ('private-token', 'private-bearer', 'private-error'):
            self.assertNotIn(secret, repr(self.events.call_args_list))
        return event

    async def check_failure(self, phase, status):
        event = await self.dispatch(dict(status=-1, usage=None, body='private-error',
                                        failure_phase=phase, head_status=status))
        self.assertEqual(event['failure_phase'], phase)
        self.assertEqual(event['head_status'], status)
        self.assertEqual(event['outcome'], 'error')
        self.assertRegex(event['job'], r'^job-[0-9a-f]{32}$')

    async def test_fetch_failure_logs_only_fixed_facts(self):
        await self.check_failure('fetch', None)

    async def test_long_stream_has_time_to_complete_inside_host_rpc_deadline(self):
        await self.dispatch(dict(status=200, usage=None, body='complete'),
                            {'stream': True, 'max_tokens': 64000})
        self.assertEqual(self.host._run_job.call_args.args[-1], 840.0)

    async def test_buffered_response_keeps_its_existing_time_limit(self):
        await self.dispatch(dict(status=200, usage=None, body='complete'),
                            {'stream': False, 'max_tokens': 64000})
        self.assertEqual(self.host._run_job.call_args.args[-1], 180.0)

    async def test_small_buffered_output_does_not_shorten_reasoning_deadline(self):
        event = await self.dispatch(dict(status=200, usage=None, body='complete'),
                            {'stream': False, 'max_output_tokens': 32})
        self.assertEqual(self.host._run_job.call_args.args[-1], 180.0)
        self.assertEqual(event['response_timeout_s'], 180.0)
        self.assertGreaterEqual(event['fetch_ms'], 0)

    async def test_small_streamed_output_keeps_the_stream_deadline(self):
        await self.dispatch(dict(status=200, usage=None, body='complete'),
                            {'stream': True, 'max_tokens': 32})
        self.assertEqual(self.host._run_job.call_args.args[-1], 840.0)

    async def test_incomplete_403_body_does_not_retry_or_cool(self):
        await self.check_failure('body', 403)

    async def test_incomplete_200_body_is_an_error(self):
        await self.check_failure('body', 200)

    async def test_legacy_failure_does_not_invent_a_phase(self):
        event = await self.dispatch(dict(status=-1, usage=None, body='private-error'))
        self.assertNotIn('failure_phase', event)
        self.assertNotIn('head_status', event)

    async def test_completed_response_discards_supplied_failure_fields(self):
        event = await self.dispatch(dict(status=200, usage=None, body='private-error',
                                        failure_phase='body', head_status=403))
        self.assertNotIn('failure_phase', event)
        self.assertNotIn('head_status', event)

    def test_malformed_diagnostics_are_not_exported(self):
        project = self.host._take_ai_post_failure_facts
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
