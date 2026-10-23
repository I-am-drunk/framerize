import http.client
import socket
import unittest
from native_check_fixture import await_state, event, terminal
from parallel_native_check.streaming import Stream
from parallel_native_check.support import Refusal


class StreamingTests(unittest.TestCase):
    def stream(self, body, framing, eof=True):
        local, remote = socket.socketpair()
        local.settimeout(2)
        remote.settimeout(2)
        self.addCleanup(remote.close)
        connection = http.client.HTTPConnection('inert.invalid')
        connection.sock = local
        connection.request('GET', '/')
        remote.recv(8192)
        remote.sendall(b'HTTP/1.1 200 OK\r\nConnection: close\r\n'
                       b'Content-Type: text/event-stream\r\n' + framing + b'\r\n\r\n' + body)
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
