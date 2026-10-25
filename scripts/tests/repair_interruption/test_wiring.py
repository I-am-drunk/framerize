import http.client
import socket
import time
import unittest
from streaming import Stream
from criteria import ready_to_disconnect
from test_acceptance import ROOT

class WiringTests(unittest.TestCase):
    def test_actual_http_header_and_generated_delta_precede_disconnect(self):
        client, peer = socket.socketpair()
        client.settimeout(2)
        peer.settimeout(2)
        self.addCleanup(peer.close)
        connection = http.client.HTTPConnection('fixture.invalid')
        connection.sock = client
        connection.request('POST', '/fixture', body=b'')
        peer.recv(4096)
        data = b'data: {"type":"response.output_text.delta","delta":"OK"}\n\n'
        peer.sendall(('HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n'
                      'Transfer-Encoding: chunked\r\nConnection: close\r\n'
                      'x-framerize-operation-id: ' + ROOT + '\r\n\r\n').encode()
                     + ('%x\r\n' % len(data)).encode() + data + b'\r\n')
        stream = Stream(connection)
        self.addCleanup(stream.close)
        deadline = time.monotonic() + 2
        while not stream.snapshot()['text_characters']:
            self.assertLess(time.monotonic(), deadline)
            time.sleep(.005)
        self.assertEqual(stream.snapshot()['operation_id'], ROOT)
        self.assertTrue(ready_to_disconnect(stream.snapshot()))
        stream.close(cancel=True)
        self.assertEqual(peer.recv(1), b'')
        self.assertFalse(stream.reader.is_alive())
