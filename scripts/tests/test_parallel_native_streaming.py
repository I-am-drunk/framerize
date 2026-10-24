import http.client
import json
import socket
import unittest
from native_check_fixture import await_state, event, terminal
from parallel_native_check.streaming import Stream
from parallel_native_check.support import Refusal


class StreamingTests(unittest.TestCase):
    @staticmethod
    def security_error():
        return {'error': {'status': 403,
            'message': 'This request was blocked by our security checks',
            'data': {'reason': 'request_blocked'},
            'ref': 'private-marker-reference', 'timestamp': 'private-marker-timestamp'}}

    def stream(self, body, framing, eof=True, status=b'200 OK', content_type=b'text/event-stream'):
        local, remote = socket.socketpair()
        local.settimeout(2)
        remote.settimeout(2)
        self.addCleanup(remote.close)
        connection = http.client.HTTPConnection('inert.invalid')
        connection.sock = local
        connection.request('GET', '/')
        remote.recv(8192)
        remote.sendall(b'HTTP/1.1 ' + status + b'\r\nConnection: close\r\n'
                       b'Content-Type: ' + content_type + b'\r\n' + framing + b'\r\n\r\n' + body)
        if eof:
            remote.shutdown(socket.SHUT_WR)
        stream = Stream(connection)
        self.addCleanup(stream.close)
        return stream, connection, remote

    def test_complete_stream_requires_clean_http_eof(self):
        wire = event('response.output_text.delta', delta='OK') + terminal()
        stream, _, _ = self.stream(wire, b'Content-Length: ' + str(len(wire)).encode())
        state = await_state(stream, lambda state: state['finished'])
        self.assertTrue(state['eof'])
        self.assertIsNone(state['read_error'])
        self.assertEqual(state['terminal_completions'], 1)

    def test_preflight_error_retains_only_allowlisted_code(self):
        body = b'{"error":{"type":"native_dispatch_error","code":"BodyTooLarge","message":"private-marker"}}'
        stream, _, _ = self.stream(body, b'Content-Length: ' + str(len(body)).encode(),
                                  status=b'502 Bad Gateway', content_type=b'application/json')
        value = await_state(stream, lambda state: state['finished'])
        self.assertEqual(value['status'], 502)
        self.assertEqual(value['response_error_code'], 'BodyTooLarge')
        self.assertEqual(value['read_error'], 'stream_http_status')
        self.assertFalse(value['eof'])
        self.assertNotIn('private-marker', str(value))

    def test_error_response_cannot_export_unknown_codes_or_large_bodies(self):
        for body in [
            b'{"error":{"type":"native_dispatch_error","code":"private-marker"}}',
            b'{"error":{"type":"provider_error","code":"BodyTooLarge"}}',
            b'{"error":{"type":"native_dispatch_error","code":"BodyTooLarge"},"padding":"'
            + b'x' * 8192 + b'"}',
            b'{"error":"private-marker"}',
        ]:
            with self.subTest(body_length=len(body)):
                stream, _, _ = self.stream(body, b'Content-Length: ' + str(len(body)).encode(),
                                          status=b'502 Bad Gateway', content_type=b'application/json')
                value = await_state(stream, lambda state: state['finished'])
                self.assertIsNone(value['response_error_code'])
                self.assertEqual(value['read_error'], 'stream_http_status')
                self.assertNotIn('private-marker', str(value))

    def test_security_error_retains_category_without_private_details(self):
        body = json.dumps(self.security_error()).encode()
        stream, _, _ = self.stream(body, b'Content-Length: ' + str(len(body)).encode(),
                                  status=b'403 Forbidden', content_type=b'application/json')
        value = await_state(stream, lambda state: state['finished'])
        details = value['response_failure']
        self.assertEqual(set(details), {'native_error_code', 'content_kind', 'json_valid',
            'json_bytes', 'json_truncated', 'framer_security_envelope', 'body_framing'})
        self.assertTrue(details['framer_security_envelope'])
        self.assertTrue(details['json_valid'])
        self.assertEqual(details['body_framing'], 'complete')
        self.assertEqual(details['json_bytes'], len(body))
        self.assertIsNone(value['response_error_code'])
        self.assertEqual(value['read_error'], 'stream_http_status')
        self.assertNotIn('private-marker', str(value))

    def test_security_category_requires_complete_envelope_and_http_403(self):
        for missing in ('status', 'message', 'data', 'ref', 'timestamp', None):
            with self.subTest(missing=missing):
                value = self.security_error()
                if missing:
                    del value['error'][missing]
                body = json.dumps(value).encode()
                status = b'403 Forbidden' if missing else b'500 Server Error'
                stream, _, _ = self.stream(body, b'Content-Length: ' + str(len(body)).encode(),
                                          status=status, content_type=b'application/json')
                value = await_state(stream, lambda state: state['finished'])
                self.assertFalse(value['response_failure']['framer_security_envelope'])
                self.assertEqual(value['read_error'], 'stream_http_status')
                self.assertNotIn('private-marker', str(value))

    def test_non_json_error_retains_only_media_category(self):
        body = b'private-marker'
        for media, category in [(b'text/html', 'html'), (b'private-marker', 'other')]:
            stream, _, _ = self.stream(body, b'Content-Length: ' + str(len(body)).encode(),
                                      status=b'403 Forbidden', content_type=media)
            value = await_state(stream, lambda state: state['finished'])
            self.assertEqual(value['response_failure']['content_kind'], category)
            self.assertIsNone(value['response_failure']['json_bytes'])
            self.assertEqual(value['response_failure']['body_framing'], 'unobserved')
            self.assertFalse(value['response_failure']['framer_security_envelope'])
            self.assertNotIn('private-marker', str(value))

    def test_error_json_validity_is_separate_from_http_completeness(self):
        body = b'{"error":{"type":"native_dispatch_error","code":"Body"}}'
        for extra, expected in [(0, 'complete'), (1, 'incomplete')]:
            with self.subTest(extra_bytes=extra):
                stream, _, _ = self.stream(body,
                    b'Content-Length: ' + str(len(body) + extra).encode(),
                    status=b'502 Bad Gateway', content_type=b'application/json')
                value = await_state(stream, lambda state: state['finished'])
                details = value['response_failure']
                self.assertEqual(details['body_framing'], expected)
                self.assertTrue(details['json_valid'])
                self.assertFalse(details['json_truncated'])
                self.assertEqual(details['native_error_code'], 'Body')
                self.assertEqual(value['read_error'], 'stream_http_status')
                self.assertFalse(value['eof'])

    def test_error_framing_covers_chunked_and_close_delimited_bodies(self):
        body = b'{"error":{"type":"native_dispatch_error","code":"Body"}}'
        chunk = f'{len(body):x}\r\n'.encode() + body + b'\r\n'
        cases = [(body, b'', 'complete'),
            (chunk + b'0\r\n\r\n', b'Transfer-Encoding: chunked', 'complete'),
            (chunk, b'Transfer-Encoding: chunked', 'incomplete')]
        for wire, framing, expected in cases:
            with self.subTest(framing=framing, expected=expected):
                stream, _, _ = self.stream(wire, framing, status=b'502 Bad Gateway',
                    content_type=b'application/json')
                value = await_state(stream, lambda state: state['finished'])
                details = value['response_failure']
                self.assertEqual(details['body_framing'], expected)
                self.assertTrue(details['json_valid'])
                self.assertEqual(details['native_error_code'], 'Body')
                self.assertEqual(value['read_error'], 'stream_http_status')
                self.assertFalse(value['eof'])

    def test_error_capture_limit_does_not_claim_http_incompleteness(self):
        body = b' ' * 8193
        for length, expected in [(8193, 'complete'), (8194, 'unobserved')]:
            stream, _, _ = self.stream(body, b'Content-Length: ' + str(length).encode(),
                status=b'502 Bad Gateway', content_type=b'application/json')
            value = await_state(stream, lambda state: state['finished'])
            self.assertEqual(value['response_failure']['body_framing'], expected)
            self.assertTrue(value['response_failure']['json_truncated'])
            self.assertFalse(value['response_failure']['json_valid'])
            self.assertEqual(value['read_error'], 'stream_http_status')

    def test_cancel_shuts_down_detached_socket_after_real_text(self):
        frame = event('response.output_text.delta', delta='started')
        chunk = f'{len(frame):x}\r\n'.encode() + frame + b'\r\n'
        stream, connection, peer = self.stream(chunk, b'Transfer-Encoding: chunked', False)
        await_state(stream, lambda value: value['text_characters'] > 0)
        self.assertIsNone(connection.sock)
        stream.close(cancel=True)
        self.assertEqual(peer.recv(1), b'')
        state = stream.snapshot()
        self.assertTrue(state['finished'])
        self.assertFalse(state['eof'])
        self.assertIn('disconnected_at_unix', state)

    def test_completion_cannot_be_reported_as_cancellation(self):
        wire = terminal()
        stream, _, _ = self.stream(wire, b'Content-Length: ' + str(len(wire)).encode())
        await_state(stream, lambda value: value['finished'])
        with self.assertRaisesRegex(Refusal, 'stream_finished_before_disconnect'):
            stream.close(cancel=True)
        self.assertNotIn('disconnected_at_unix', stream.snapshot())

    def test_truncated_content_length_rejects_even_terminal_event(self):
        wire = terminal()
        stream, _, _ = self.stream(wire, b'Content-Length: ' + str(len(wire) + 10).encode())
        value = await_state(stream, lambda value: value['finished'])
        self.assertEqual(value['read_error'], 'stream_http_framing')
        self.assertFalse(value['eof'])

    def test_truncated_chunk_is_not_clean_eof(self):
        stream, _, _ = self.stream(b'20\r\nshort', b'Transfer-Encoding: chunked')
        value = await_state(stream, lambda value: value['finished'])
        self.assertIsNotNone(value['read_error'])
        self.assertFalse(value['eof'])

    def test_parse_failure_report_excludes_raw_payload(self):
        wire = b'data: private-provider-value\n\n'
        stream, _, _ = self.stream(wire, b'Content-Length: ' + str(len(wire)).encode())
        value = await_state(stream, lambda value: value['finished'])
        self.assertEqual(value['read_error'], 'JSONDecodeError')
        self.assertNotIn('private-provider-value', str(value))
