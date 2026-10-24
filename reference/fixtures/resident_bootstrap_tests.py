"""Inert owner-bootstrap tests; fake RPCs only, no browser or SDK."""
import asyncio
import contextlib
import io
import json
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import resident_authorization_tests as fixture
import resident_bootstrap as bootstrap


class ResidentBootstrapTests(unittest.IsolatedAsyncioTestCase):
    async def test_missing_provenance_has_bound_no_work_receipt(self):
        self.acc._completed_browser_http200 = None
        result = await bootstrap.command(self.host, self.request)
        self.assertEqual(result, dict(self.request, status='unavailable',
            stage='provenance', browser_settled=True))
        self.assertIsNone(self.life.activity.current)
        self.assertFalse(self.life.activity.fenced)
        self.assertFalse(getattr(self.host, '_resident_bootstrap_tasks', set()))
        self.assertEqual((len(self.page.calls), len(self.context.calls)), (0, 0))

    async def test_private_rpc_error_is_not_logged(self):
        self.page.error = RuntimeError('synthetic-private-detail')
        output = io.StringIO()
        with contextlib.redirect_stderr(output):
            with self.assertRaises(fixture.ResidentRefused):
                await bootstrap.command(self.host, self.request)
        self.assertEqual(json.loads(output.getvalue()), {
            'event': 'resident.bootstrap.refused', 'stage': 'internal'})
        self.assertTrue(self.life.activity.fenced)

    async def asyncSetUp(self):
        await fixture.ResidentAuthorizationTests.asyncSetUp(self)
        self.host.proxy_revisions = {}
        self.request = dict(version=2, account=fixture.ACCOUNT,
            application_generation=7, bootstrap=fixture.LEASE,
            project='project', path='/ai/v3/openai/responses/', proxy_revision=0)
        self.data = dict(ok=True, page_url=fixture.URL, device='synthetic-device',
                         wrapper_url=fixture.WRAPPER, wrapper_export='a')

        async def evaluate(source):
            self.assertEqual(source, bootstrap.READ_JS)
            self.page.calls.append(source)
            self.page.entered.set()
            await self.page.release.wait()
            if self.page.hook:
                self.page.hook()
            if self.page.error:
                raise self.page.error
            return dict(self.data)
        self.page.evaluate = evaluate

    async def asyncTearDown(self):
        self.page.release.set()
        await asyncio.gather(*tuple(getattr(self.host, '_resident_bootstrap_tasks', ())),
                             return_exceptions=True)
        await fixture.ResidentAuthorizationTests.asyncTearDown(self)

    async def test_bootstrap_returns_metadata_without_mint_or_credentials(self):
        result = await bootstrap.command(self.host, self.request)
        self.assertEqual(set(result), set(self.request) | {
            'host_generation', 'account_incarnation', 'provenance', 'page_url',
            'device', 'effective_proxy', 'wrapper_url', 'wrapper_export', 'browser_settled'})
        self.assertEqual(result['page_url'], fixture.URL)
        self.assertTrue(result['browser_settled'])
        self.assertIsNone(self.life.activity.current)
        self.assertFalse(self.life.activity.fenced)
        self.assertEqual((len(self.page.calls), len(self.context.calls)), (1, 2))

    async def test_unknown_fields_and_wrong_revision_refuse_before_rpc(self):
        for request in [dict(self.request, secret='extra'),
                        dict(self.request, proxy_revision=1),
                        dict(self.request, application_generation=8)]:
            with self.assertRaises(fixture.ResidentRefused):
                await bootstrap.command(self.host, request)
        self.assertEqual((len(self.page.calls), len(self.context.calls)), (0, 0))

    async def test_missing_provenance_never_selects_another_page(self):
        self.acc._completed_browser_http200 = None
        result = await bootstrap.command(self.host, self.request)
        self.assertEqual(result['status'], 'unavailable')
        self.assertEqual((len(self.page.calls), len(self.context.calls)), (0, 0))

    async def test_busy_account_refuses_without_reading_page(self):
        activity = self.life.activity.activity()
        try:
            with self.assertRaises(fixture.ResidentRefused):
                await bootstrap.command(self.host, self.request)
        finally:
            activity.close()
        self.assertEqual(len(self.page.calls), 0)

    async def test_page_drift_cannot_publish_bootstrap(self):
        self.page.hook = lambda: setattr(self.page, 'url', fixture.URL + '?changed')
        with self.assertRaises(fixture.ResidentRefused):
            await bootstrap.command(self.host, self.request)
        self.assertEqual(len(self.page.calls), 1)

    async def test_rpc_failure_fences_without_exporting_details(self):
        self.page.error = RuntimeError('synthetic-private-detail')
        with self.assertRaisesRegex(fixture.ResidentRefused, '^resident bootstrap refused$'):
            await bootstrap.command(self.host, self.request)
        self.assertTrue(self.life.activity.fenced)
        self.assertEqual(len(self.page.calls), 1)

    async def test_malformed_wrapper_reply_refuses(self):
        self.data['extra'] = 'synthetic-private-value'
        with self.assertRaises(fixture.ResidentRefused):
            await bootstrap.command(self.host, self.request)
        self.assertEqual(len(self.page.calls), 1)

    async def test_lost_waiter_keeps_raw_rpc_and_lease_until_settlement(self):
        self.page.release.clear()
        waiter = asyncio.create_task(bootstrap.command(self.host, self.request))
        await self.page.entered.wait()
        lease = self.life.activity.current
        raw = next(iter(self.life.activity.pending))
        waiter.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await waiter
        self.assertFalse(raw.done())
        self.assertIs(self.life.activity.current, lease)
        owners = tuple(self.host._resident_bootstrap_tasks)
        self.assertEqual(len(owners), 1)
        self.page.release.set()
        await asyncio.gather(*owners)
        self.assertTrue(raw.done())
        self.assertIsNone(self.life.activity.current)

    async def test_late_provenance_loss_is_not_a_no_work_receipt(self):
        self.page.hook = self.acc._completed_browser_http200.invalidate
        with self.assertRaises(fixture.ResidentRefused):
            await bootstrap.command(self.host, self.request)
        self.assertEqual(len(self.page.calls), 1)
        self.assertIsNone(self.life.activity.current)

    async def test_actual_launch_realm_required_without_new_evaluation_mode(self):
        self.acc.resident_launch = (self.browser, 1, None, True)
        result = await bootstrap.command(self.host, self.request)
        self.assertEqual(result['status'], 'unavailable')
        self.assertEqual(len(self.page.calls), 0)


if __name__ == '__main__':
    unittest.main()
