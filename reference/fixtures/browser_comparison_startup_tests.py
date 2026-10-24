"""Passive startup telemetry: inert ports, no browser, SDK or HTTP."""
import asyncio
import importlib.util
import json
from pathlib import Path
from types import SimpleNamespace
import unittest

spec = importlib.util.spec_from_file_location('_startup_diagnostics',
    Path(__file__).with_name('browser_comparison_readiness_tests.py'))
R = importlib.util.module_from_spec(spec)
spec.loader.exec_module(R)
C = R.C
PRIVATE = 'private-startup-sentinel'
BASE = 'https://app.framerstatic.com/'


class Page:
    def __init__(self):
        self.listeners, self.main_frame = {}, object()

    def on(self, event, callback):
        self.listeners.setdefault(event, []).append(callback)

    def remove_listener(self, event, callback):
        self.listeners[event].remove(callback)

    def fire(self, event, *args):
        for callback in list(self.listeners.get(event, [])):
            callback(*args)


class Request:
    def __init__(self, url, frame=None):
        self.url, self.frame = url, frame

    def is_navigation_request(self):
        return self.frame is not None


class StartupTests(unittest.TestCase):
    def fixture(self):
        page, events, scope = Page(), [], SimpleNamespace()
        C.comparison_attach_progress(scope, {'purpose': 'baseline_context'},
            lambda event, **fields: events.append(dict(event=event, **fields)))
        return page, events, C.ComparisonStartup(scope, page)

    def test_public_module_filter_never_exports_private_urls(self):
        for name in ('web.MNQWRMGH.mjs', 'chunk-3W5IVLGF.mjs'):
            self.assertEqual(C.comparison_public_module(BASE + name), name)
        for url in (None, BASE + PRIVATE, BASE + 'web.MNQWRMGH.mjs?token=' + PRIVATE,
                    BASE + 'web.MNQWRMGH.mjs#' + PRIVATE, BASE + '../web.MNQWRMGH.mjs',
                    'http://app.framerstatic.com/web.MNQWRMGH.mjs',
                    'https://user@app.framerstatic.com/web.MNQWRMGH.mjs',
                    'https://framer.com/projects/' + PRIVATE):
            self.assertIsNone(C.comparison_public_module(url))

    def test_document_events_and_public_module_lifecycle_are_distinct(self):
        page, events, capture = self.fixture()
        document = Request('https://framer.com/projects/' + PRIVATE, page.main_frame)
        page.fire('response', SimpleNamespace(request=document, status=200))
        frame = Request(document.url, object())
        page.fire('response', SimpleNamespace(request=frame, status=403))
        page.fire('domcontentloaded', page)
        page.fire('load', page)
        web = Request(BASE + 'web.MNQWRMGH.mjs')
        policy = Request(BASE + 'chunk-3W5IVLGF.mjs')
        wrapper = Request(C.COMPARISON_WRAPPER)
        for request in (web, policy, wrapper):
            page.fire('request', request)
        page.fire('response', SimpleNamespace(request=web, status=200))
        page.fire('requestfinished', web)
        page.fire('response', SimpleNamespace(request=policy, status=503))
        page.fire('requestfinished', policy)
        page.fire('pageerror', RuntimeError(PRIVATE))
        capture.emit('navigation')
        row = events[-1]
        self.assertTrue(row['available'] and row['tracking_complete'])
        self.assertEqual((row['document_status'], row['domcontentloaded'], row['load']),
                         (200, True, True))
        self.assertEqual((row['module_started'], row['module_finished'], row['module_pending']),
                         (3, 2, 1))
        self.assertEqual((row['module_failed'], row['module_http_error'], row['page_errors']),
                         (0, 1, 1))
        self.assertTrue(row['policy_requested'] and row['wrapper_requested'])
        self.assertEqual(row['web_entries'], ['web.MNQWRMGH.mjs'])
        self.assertEqual(row['failed_modules'], ['chunk-3W5IVLGF.mjs'])
        self.assertNotIn(PRIVATE, json.dumps(events))
        capture.close()
        self.assertFalse(capture.pending)

    def test_failures_bounds_and_snapshot_lists_do_not_grow_later(self):
        page, events, capture = self.fixture()
        for index in range(600):
            request = Request(BASE + f'chunk-{index:08d}.mjs')
            page.fire('request', request)
            page.fire('requestfailed', request)
        capture.emit('finished')
        row = events[-1]
        self.assertEqual((row['module_started'], row['module_failed']), (512, 512))
        self.assertEqual(len(row['failed_modules']), 8)
        self.assertFalse(row['tracking_complete'])
        self.assertEqual(row['module_pending'], 0)
        capture.facts['failed_modules'].clear()
        self.assertEqual(len(row['failed_modules']), 8)
        capture.close()

    def test_callback_errors_and_partial_registration_stay_unknown(self):
        page, events, capture = self.fixture()
        page.fire('response', SimpleNamespace(request=None, status=PRIVATE))
        capture.emit('finished')
        self.assertFalse(events[-1]['tracking_complete'])
        capture.close()
        def fail(*_):
            raise RuntimeError(PRIVATE)
        page.on = fail
        capture = C.ComparisonStartup(capture.scope, page)
        capture.emit('finished')
        self.assertFalse(events[-1]['available'])
        self.assertNotIn(PRIVATE, json.dumps(events))
        capture.close()

    def test_close_removes_only_owned_callbacks_and_late_events_are_inert(self):
        page, events, capture = self.fixture()
        external = lambda: None
        page.on('load', external)
        retained = page.listeners['pageerror'][0]
        capture.close()
        retained(RuntimeError(PRIVATE))
        self.assertEqual(capture.facts['page_errors'], 0)
        self.assertEqual(page.listeners['load'], [external])
        self.assertTrue(all(not callbacks for event, callbacks in page.listeners.items()
                            if event != 'load'))

    def test_events_are_capped_and_sink_failure_does_not_escape(self):
        page, events, capture = self.fixture()
        for _ in range(10):
            capture.emit('finished')
        capture.emit(PRIVATE)
        self.assertEqual(len(events), 4)
        self.assertEqual([row['sequence'] for row in events], [1, 2, 3, 4])
        def fail(*_):
            raise RuntimeError(PRIVATE)
        capture.scope.comparison_startup_emit = fail
        capture.emit('finished')
        capture.close()


class StartupWorkflowTests(unittest.IsolatedAsyncioTestCase):
    asyncSetUp = R.BrowserComparisonReadinessTests.asyncSetUp
    asyncTearDown = R.BrowserComparisonReadinessTests.asyncTearDown
    fixture = R.BrowserComparisonReadinessTests.fixture

    async def test_success_records_stages_without_extra_browser_operations(self):
        value = self.fixture()
        await value.prepare_context()
        self.assertEqual([row['phase'] for row in value.startup_events],
                         ['navigation', 'readiness', 'policy', 'finished'])
        self.assertEqual((value.probes, value.mints, value.fetches), (0, 0, 0))
        self.assertNotIn(PRIVATE, json.dumps(value.startup_events))

    async def test_readiness_failure_keeps_refusal_and_has_no_extra_probe(self):
        value = self.fixture()
        value.ready_error = TimeoutError(PRIVATE)
        with self.assertRaises(RuntimeError):
            await value.prepare_context()
        self.assertEqual([row['phase'] for row in value.startup_events],
                         ['navigation', 'finished'])
        self.assertEqual((value.probes, value.mints, value.fetches), (1, 0, 0))
        self.assertEqual(value.events[-1]['stage'], 'editor_wait_timeout')

    async def test_cancelled_wait_retains_rpc_and_emits_no_late_checkpoints(self):
        value = self.fixture()
        value.ready_release.clear()
        task = asyncio.create_task(value.prepare_context())
        self.tasks.append(task)
        await value.ready_started.wait()
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertEqual([row['phase'] for row in value.startup_events],
                         ['navigation', 'finished'])
        value.ready_release.set()
        await asyncio.wait_for(value.scope.drain(), 2)
        self.assertEqual((value.probes, value.mints, value.fetches), (0, 0, 0))
        self.assertEqual(len(value.startup_events), 2)


if __name__ == '__main__':
    unittest.main()
