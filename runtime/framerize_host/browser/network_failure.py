"""Keep network failure facts for one request without retaining its payload."""
from http import HTTPStatus
import time

from ..events import error_message


class NetworkFailure:
    def __init__(self, page, url, turn_id):
        self.page, self.url, self.turn_id = page, url, turn_id
        self.status = self.failure = self.elapsed_ms = None
        self.started = time.monotonic()
        self.listeners = []

    def matches(self, request):
        try:
            if request.url != self.url or request.method != 'POST':
                return False
            data = request.post_data_json
            return isinstance(data, dict) and data.get('turnId') == self.turn_id
        except Exception:
            return False

    def response(self, response):
        if self.matches(response.request):
            status = response.status
            if type(status) is int and 100 <= status <= 599:
                self.status = status
                self.elapsed_ms = round((time.monotonic() - self.started) * 1000)

    def failed(self, request):
        if self.matches(request) and request.failure:
            self.failure = error_message(request.failure)

    def watch(self):
        if not isinstance(self.turn_id, str) or not self.turn_id:
            return self
        try:
            for event, callback in [('response', self.response),
                                    ('requestfailed', self.failed)]:
                self.page.on(event, callback)
                self.listeners.append((event, callback))
        except Exception:
            self.close()
        return self

    def close(self):
        for event, callback in self.listeners:
            try:
                self.page.remove_listener(event, callback)
            except Exception:
                pass
        self.listeners.clear()
        self.page = None

    def explain(self, result):
        if result.get('status') != -1:
            return result
        details = []
        if self.status is not None:
            phrase = HTTPStatus._value2member_map_.get(self.status)
            name = ' ' + phrase.phrase if phrase else ''
            details.append(f'browser observed upstream HTTP {self.status}{name}'
                           f' after {self.elapsed_ms / 1000:.1f}s')
            result['network_status'] = self.status
            result['network_elapsed_ms'] = self.elapsed_ms
        if self.failure:
            details.append('network cause: ' + self.failure)
            result['network_failure'] = self.failure
        if details:
            original = error_message(result.get('body', 'response unavailable'))
            result['body'] = '; '.join(details) + (
                '. The browser did not return a complete, acknowledged response. '
                'Browser error: ' + original)
        return result
