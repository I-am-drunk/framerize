"""Browser-observed failures remain tied to one request and release listeners."""
import gc
from types import SimpleNamespace
import unittest
import weakref

from framerize_host.browser.network_failure import NetworkFailure
from framerize_host.app import Host


class Page:
    def __init__(self):
        self.listeners = {}

    def on(self, event, callback):
        self.listeners.setdefault(event, []).append(callback)

    def remove_listener(self, event, callback):
        self.listeners[event].remove(callback)

    def emit(self, event, value):
        for callback in list(self.listeners.get(event, [])):
            callback(value)


class Request:
    url = 'https://api.framer.com/ai/v3/messages/'
    method = 'POST'
    failure = None

    def __init__(self, turn='first'):
        self.post_data_json = {'turnId': turn, 'request': {'content': 'private'}}


def failed():
    return dict(status=-1, usage=None, body='TypeError: NetworkError',
                failure_phase='fetch', head_status=None)


class NetworkFailureTests(unittest.TestCase):
    def test_uncaptured_body_uses_browser_request_identity(self):
        page = Page()
        observer = self.watch(page)
        request = Request()
        request.post_data_json = None
        request._impl_obj = SimpleNamespace(
            _initializer={'framerizeTurnId': 'first'})
        page.emit('response', SimpleNamespace(request=request, status=504))
        result = observer.explain(failed())
        self.assertIn('HTTP 504 Gateway Timeout', result['body'])

    def watch(self, page, turn='first'):
        observer = NetworkFailure(page, Request.url, turn).watch()
        self.addCleanup(observer.close)
        return observer

    def test_uncaptured_concurrent_requests_remain_separate(self):
        page = Page()
        first, second = self.watch(page), self.watch(page, 'second')
        request = Request()
        request.post_data_json = None
        request._impl_obj = SimpleNamespace(
            _initializer={'framerizeTurnId': 'second'})
        request.failure = 'NS_ERROR_NET_RESET'
        page.emit('requestfailed', request)
        self.assertEqual(first.explain(failed()), failed())
        self.assertIn('NS_ERROR_NET_RESET', second.explain(failed())['body'])

    def test_identity_does_not_bypass_origin_or_method_matching(self):
        page = Page()
        observer = self.watch(page)
        for change in ({'url': 'https://other.invalid/'}, {'method': 'OPTIONS'}):
            request = Request()
            request.post_data_json = None
            request._impl_obj = SimpleNamespace(
                _initializer={'framerizeTurnId': 'first'})
            request.__dict__.update(change)
            page.emit('response', SimpleNamespace(request=request, status=504))
        self.assertEqual(observer.explain(failed()), failed())

    def test_hidden_gateway_timeout_reports_actual_status_without_success(self):
        page = Page()
        observer = self.watch(page)
        page.emit('response', SimpleNamespace(request=Request(), status=504))
        result = observer.explain(failed())
        self.assertEqual(result['status'], -1)
        self.assertIn('HTTP 504 Gateway Timeout', result['body'])
        self.assertIn('NetworkError', result['body'])
        facts = Host._take_ai_post_failure_facts(result, 'fixture-job')
        self.assertEqual(facts['network_status'], 504)
        self.assertEqual(facts['failure_phase'], 'fetch')
        self.assertNotIn('network_status', result)

    def test_concurrent_turns_do_not_share_failure_evidence(self):
        page = Page()
        first, second = self.watch(page), self.watch(page, 'second')
        page.emit('response', SimpleNamespace(request=Request(), status=504))
        self.assertIn('504', first.explain(failed())['body'])
        self.assertEqual(second.explain(failed()), failed())
        first.close()
        page.emit('response', SimpleNamespace(request=Request('second'), status=502))
        self.assertIn('502', second.explain(failed())['body'])

    def test_other_origin_method_and_turn_cannot_supply_the_cause(self):
        page = Page()
        observer = self.watch(page)
        for change in [{'url': 'https://other.invalid/ai/v3/messages/'},
                       {'method': 'OPTIONS'}, {'post_data_json': {'turnId': 'other'}}]:
            request = Request()
            request.__dict__.update(change)
            page.emit('response', SimpleNamespace(request=request, status=504))
        self.assertEqual(observer.explain(failed()), failed())

    def test_completed_response_and_incomplete_body_remain_distinct(self):
        page = Page()
        observer = self.watch(page)
        page.emit('response', SimpleNamespace(request=Request(), status=200))
        complete = dict(status=200, usage=None, body='complete')
        self.assertEqual(observer.explain(complete.copy()), complete)
        incomplete = observer.explain(failed())
        self.assertEqual(incomplete['status'], -1)
        self.assertIn('HTTP 200 OK', incomplete['body'])

    def test_network_failure_redacts_credentials(self):
        page = Page()
        observer = self.watch(page)
        request = Request()
        request.failure = 'NS_ERROR_NET_TIMEOUT Authorization: Bearer private-value'
        page.emit('requestfailed', request)
        result = observer.explain(failed())
        self.assertIn('NS_ERROR_NET_TIMEOUT', result['body'])
        self.assertNotIn('private-value', repr(result))

    def test_repeated_requests_retain_neither_payloads_nor_listeners(self):
        page = Page()
        for index in range(512):
            observer = NetworkFailure(page, Request.url, 'first').watch()
            request = Request()
            if index % 2:
                request.post_data_json = None
                request._impl_obj = SimpleNamespace(
                    _initializer={'framerizeTurnId': 'first'})
            retained = weakref.ref(request)
            page.emit('response', SimpleNamespace(request=request, status=504))
            del request
            self.assertIsNone(retained())
            observer.close()
            self.assertIsNone(observer.page)
            self.assertTrue(all(not values for values in page.listeners.values()))
        del observer
        gc.collect()

    def test_missing_request_identity_cannot_attach_unscoped_observation(self):
        page = Page()
        observer = self.watch(page, None)
        self.assertFalse(page.listeners)
        self.assertEqual(observer.explain(failed()), failed())
