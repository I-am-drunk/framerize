import http.client
import socket
import time
import unittest
from types import SimpleNamespace
from unittest.mock import patch
from native_check_fixture import await_state, event, terminal
from parallel_native_check import client
from parallel_native_check.streaming import Stream


class TimingTests(unittest.TestCase):
    def open_stream(self, request_started=None):
        local, peer = socket.socketpair()
        local.settimeout(2)
        peer.settimeout(2)
        self.addCleanup(peer.close)
        connection = http.client.HTTPConnection('inert.invalid')
        connection.sock = local
        connection.request('GET', '/')
        peer.recv(8192)
        peer.sendall(b'HTTP/1.1 200 OK\r\nConnection: close\r\n'
                     b'Content-Type: text/event-stream\r\n'
                     b'Transfer-Encoding: chunked\r\n\r\n')
        stream = Stream(connection, request_started=request_started)
        self.addCleanup(stream.close)
        return stream, peer

    @staticmethod
    def chunk(peer, data):
        peer.sendall(f'{len(data):x}\r\n'.encode() + data + b'\r\n')

    def test_headers_heartbeat_and_partial_text_do_not_start_text_latency(self):
        stream, peer = self.open_stream()
        headers = await_state(stream, lambda state: state['status'] == 200)
        self.assertIsNone(headers['first_text_elapsed_ms'])
        heartbeat = b': keepalive\n\n'
        self.chunk(peer, heartbeat)
        await_state(stream, lambda state: state['bytes'] >= len(heartbeat))
        delta = event('response.output_text.delta', delta='A')
        self.chunk(peer, delta[:-1])
        partial = await_state(stream, lambda state: state['bytes'] >= len(heartbeat + delta) - 1)
        self.assertIsNone(partial['first_text_elapsed_ms'])
        self.chunk(peer, delta[-1:])
        first = await_state(stream, lambda state: state['text_characters'] == 1)
        self.assertGreaterEqual(first['first_text_elapsed_ms'], headers['headers_elapsed_ms'])
        self.assertIsNone(first['terminal_elapsed_ms'])
        self.chunk(peer, event('response.output_text.delta', delta='B'))
        second = await_state(stream, lambda state: state['text_characters'] == 2)
        self.assertEqual(second['first_text_elapsed_ms'], first['first_text_elapsed_ms'])
        self.chunk(peer, terminal('AB'))
        complete = await_state(stream, lambda state: state['terminal_completions'] == 1)
        self.assertGreaterEqual(complete['terminal_elapsed_ms'], first['first_text_elapsed_ms'])
        self.assertFalse(complete['finished'])
        self.assertIsNone(complete['reader_finished_elapsed_ms'])
        peer.sendall(b'0\r\n\r\n')
        finished = await_state(stream, lambda state: state['finished'])
        self.assertTrue(finished['eof'])
        self.assertIsNone(finished['read_error'])
        self.assertGreaterEqual(finished['reader_finished_elapsed_ms'], complete['terminal_elapsed_ms'])

    def test_supplied_request_start_and_terminal_text_survive_wall_clock_change(self):
        stream, peer = self.open_stream((time.monotonic() - 1, 1700000000.0))
        headers = await_state(stream, lambda state: state['status'] == 200)
        self.assertEqual(headers['request_started_at_unix'], 1700000000.0)
        self.assertGreaterEqual(headers['headers_elapsed_ms'], 1000)
        with patch('parallel_native_check.streaming.time.time', return_value=-5000):
            self.chunk(peer, terminal())
            peer.sendall(b'0\r\n\r\n')
            state = await_state(stream, lambda value: value['finished'])
        self.assertTrue(state['eof'])
        self.assertIsNone(state['read_error'])
        self.assertEqual(state['first_text_elapsed_ms'], state['terminal_elapsed_ms'])
        self.assertGreaterEqual(state['first_text_elapsed_ms'], headers['headers_elapsed_ms'])
        self.assertGreaterEqual(state['reader_finished_elapsed_ms'], state['terminal_elapsed_ms'])

    def test_parse_failure_has_no_success_timing_or_private_payload(self):
        stream, peer = self.open_stream()
        self.chunk(peer, b'data: private-provider-value\n\n')
        state = await_state(stream, lambda value: value['finished'])
        self.assertEqual(state['read_error'], 'JSONDecodeError')
        self.assertIsNone(state['first_text_elapsed_ms'])
        self.assertIsNone(state['terminal_elapsed_ms'])
        self.assertGreaterEqual(state['reader_finished_elapsed_ms'], state['headers_elapsed_ms'])
        self.assertNotIn('private-provider-value', str(state))

    def test_cancel_retains_first_text_and_distinguishes_reader_from_completion(self):
        stream, peer = self.open_stream()
        self.chunk(peer, event('response.output_text.delta', delta='started'))
        first = await_state(stream, lambda state: state['text_characters'] > 0)
        stream.close(cancel=True)
        self.assertEqual(peer.recv(1), b'')
        state = stream.snapshot()
        self.assertEqual(state['first_text_elapsed_ms'], first['first_text_elapsed_ms'])
        self.assertGreaterEqual(state['disconnected_elapsed_ms'], state['first_text_elapsed_ms'])
        self.assertGreaterEqual(state['reader_finished_elapsed_ms'], state['disconnected_elapsed_ms'])
        self.assertIsNone(state['terminal_elapsed_ms'])
        self.assertFalse(state['eof'])

    def test_send_captures_start_before_connection_request(self):
        binding = SimpleNamespace(account='synthetic-account', input={'context': {}})
        report = {'post_calls': 0}
        with patch.object(client.http.client, 'HTTPConnection') as connection, \
                patch.object(client, 'Stream') as reader, \
                patch.object(client.time, 'monotonic', return_value=10.0) as clock, \
                patch.object(client.time, 'time', return_value=1700000000.0):
            def request(*args, **kwargs):
                clock.return_value = 20.0
            connection.return_value.request.side_effect = request
            result = client.send(binding, False, report)
            reader.assert_called_once_with(connection.return_value,
                                           request_started=(10.0, 1700000000.0))
            self.assertIs(result, reader.return_value)
        self.assertEqual(report['post_calls'], 1)
