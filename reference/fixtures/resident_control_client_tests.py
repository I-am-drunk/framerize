"""Inert client fixtures. No browser, SDK, daemon or network connection."""
import importlib.util
import json
import os
from pathlib import Path
import struct
import tempfile
import time
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('resident_client',
    ROOT / 'scripts/resident_control_client.py')
client = importlib.util.module_from_spec(spec)
spec.loader.exec_module(client)


def report():
    value = {key: False for key in client.BOOLEANS}
    value.update({key: None for key in client.ENUMS.keys() | client.STATUSES})
    return value


def framed(value):
    data = json.dumps(value).encode()
    return struct.pack('!I', len(data)) + data


class Peer:
    def __init__(self, reply=b''):
        self.reply, self.sent, self.read_sizes = reply, [], []
    def settimeout(self, value):
        self.timeout = value
    def sendall(self, value):
        self.sent.append(value)
    def recv(self, size):
        self.read_sizes.append(size)
        result, self.reply = self.reply[:size], self.reply[size:]
        return result
    def __enter__(self):
        return self
    def __exit__(self, *args):
        pass
    def connect(self, path):
        pass
    def getsockopt(self, *args):
        return struct.pack('3i', 123, os.geteuid(), os.getegid())


class ClientTests(unittest.TestCase):
    def exchange(self, peer):
        return client.exchange(peer, b'fixture request', time.monotonic() + 1)

    def test_one_send_and_complete_report_without_inference_success_claim(self):
        value = report()
        value.update(failure='baseline', baseline_status=403, cleanup_confirmed=True)
        peer = Peer(framed(value))
        self.assertEqual(self.exchange(peer), value)
        self.assertEqual(peer.sent, [b'fixture request'])
        self.assertEqual(peer.read_sizes[-1], 1)

    def test_model_output_fact_preserves_later_cleanup_failure(self):
        value = report()
        value.update(native_output_complete=True, native_invoked=True, status=200,
                     body_complete=True, json_object=True, failure='cleanup')
        self.assertEqual(self.exchange(Peer(framed(value))), value)
        self.assertFalse(value['cleanup_confirmed'])

    def test_model_output_fact_requires_matching_response_facts(self):
        for key, bad in [('native_invoked', False), ('status', 403),
                         ('body_complete', False), ('json_object', False)]:
            value = report()
            value.update(native_output_complete=True, native_invoked=True, status=200,
                         body_complete=True, json_object=True)
            value[key] = bad
            with self.assertRaises(ValueError):
                self.exchange(Peer(framed(value)))

    def test_partial_send_failure_is_never_retried(self):
        class Partial(Peer):
            def sendall(self, value):
                self.sent.append(value[:2])
                raise TimeoutError('private exception marker')
        peer = Partial()
        with self.assertRaises(TimeoutError):
            self.exchange(peer)
        self.assertEqual(len(peer.sent), 1)
        self.assertEqual(peer.read_sizes, [])

    def test_invalid_lengths_and_truncated_replies_never_retry(self):
        for reply in (b'', b'\x00\x00', struct.pack('!I', 0),
                      struct.pack('!I', client.MAX_PAYLOAD + 1), struct.pack('!I', 5) + b'{}'):
            peer = Peer(reply)
            with self.assertRaises(ValueError):
                self.exchange(peer)
            self.assertEqual(len(peer.sent), 1)

    def test_report_rejects_extra_private_fields_and_invalid_fact_types(self):
        for key, bad in [('token', 'private-token'), ('status', True),
                         ('status', 200.0), ('cleanup_confirmed', 1),
                         ('failure', 'private-error'), ('refusal_policy', []),
                         ('native_output_complete', 1)]:
            value = report()
            value[key] = bad
            with self.assertRaises(ValueError):
                self.exchange(Peer(framed(value)))

    def test_duplicate_report_keys_and_trailing_frames_are_refused(self):
        with self.assertRaises(ValueError):
            client.checked_report(b'{"status":200,"status":403}')
        with self.assertRaises(ValueError):
            self.exchange(Peer(framed(report()) + b'private-extra-frame'))

    def test_model_output_fact_is_required_in_current_wire_schema(self):
        value = report()
        del value['native_output_complete']
        with self.assertRaises(ValueError):
            self.exchange(Peer(framed(value)))

    def test_absolute_deadline_prevents_any_send(self):
        peer = Peer(framed(report()))
        with self.assertRaises(TimeoutError):
            client.exchange(peer, b'fixture', time.monotonic() - 1)
        self.assertEqual(peer.sent, [])

    def test_input_rejects_missing_context_and_nested_credentials(self):
        value = dict.fromkeys(client.INPUT, 'fixture')
        value['request'] = {'stream': False, 'input': 'fixture'}
        self.assertTrue(client.checked_request(json.dumps(value).encode()))
        value['request']['nested'] = [{'Authoriz\u0061tion': 'private-token'}]
        with self.assertRaises(ValueError):
            client.checked_request(json.dumps(value).encode())
        with self.assertRaises(ValueError):
            client.checked_request(b'{}')

    def test_run_redacts_transport_errors_and_distinguishes_unsent_from_unknown(self):
        for method, expected, code in [('connect', 'not_started', 2), ('sendall', 'unknown', 3)]:
            peer = Peer()
            with patch.object(client, 'private_request', return_value=b'frame'), \
                 patch.object(client, 'socket_identity', return_value=(1, 2, 3, 4)), \
                 patch.object(client.socket, 'socket', return_value=peer), \
                 patch.object(peer, method, side_effect=OSError('private-credential-marker')):
                result, actual = client.run('/fixture', '/fixture.json', 1)
            self.assertEqual(result, {'transport': expected, 'report': None})
            self.assertEqual(actual, code)
            self.assertNotIn('private-credential-marker', json.dumps(result))

    def test_foreign_peer_refuses_before_sending(self):
        peer = Peer()
        with patch.object(client, 'private_request', return_value=b'frame'), \
             patch.object(client, 'socket_identity', return_value=(1, 2, 3, 4)), \
             patch.object(client.socket, 'socket', return_value=peer), \
             patch.object(peer, 'getsockopt', return_value=struct.pack('3i', 123, os.geteuid() + 1, 0)):
            result, code = client.run('/fixture', '/fixture.json', 1)
        self.assertEqual((result['transport'], code, peer.sent), ('not_started', 2, []))

    def test_request_file_requires_private_regular_file_without_symlink(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / 'input.json'
            value = dict.fromkeys(client.INPUT, 'fixture')
            value['request'] = {'input': 'fixture'}
            path.write_text(json.dumps(value))
            path.chmod(0o600)
            self.assertTrue(client.private_request(path))
            link = Path(root) / 'link.json'
            link.symlink_to(path)
            with self.assertRaises(OSError):
                client.private_request(link)
            path.chmod(0o644)
            with self.assertRaises(ValueError):
                client.private_request(path)


if __name__ == '__main__':
    unittest.main()
