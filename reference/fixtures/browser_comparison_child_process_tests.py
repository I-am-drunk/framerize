"""Inert tests of the real child executable. No SDK, browser or remote HTTP."""
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import tempfile
import unittest

BINARY = None


def send(sock, value):
    body = json.dumps(value).encode()
    sock.sendall(struct.pack('!I', len(body)) + body)


def receive(sock):
    def exact(size):
        data = b''
        while len(data) < size:
            part = sock.recv(size - len(data))
            if not part:
                raise EOFError('short inert child response')
            data += part
        return data
    size = struct.unpack('!I', exact(4))[0]
    if not 0 < size <= 8192:
        raise ValueError('unbounded inert child response')
    return json.loads(exact(size))


class ChildProcessTests(unittest.TestCase):
    def setUp(self):
        if BINARY is None:
            self.skipTest('requires explicit built child executable')
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.script = self.root / 'observed_sidecar.py'
        self.script.write_text("raise AssertionError('inert child must not launch host')\n")
        self.parent, child = socket.socketpair()
        self.addCleanup(self.parent.close)
        self.parent.settimeout(4)
        try:
            self.process = subprocess.Popen([str(BINARY), '--control-fd', str(child.fileno())],
                pass_fds=(child.fileno(),), stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        finally:
            child.close()
        self.addCleanup(self.reap)

    def reap(self):
        if self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=2)
        self.process.stdout.close()
        self.process.stderr.close()

    def plan(self):
        return dict(version=1, nonce='a' * 32,
            account='00000000-0000-4000-8000-000000000001',
            user='fixture-user', team='fixture-team', project='fixture-project',
            chat_session='fixture-session', route='responses',
            payload=dict(model='fixture-model', stream=False, input='fixture'),
            context=dict(feature_category='fixture', feature_name='fixture', step='fixture',
                         turn_id='native-turn', user_prompt='fixture'),
            baseline_turn='browser-turn', token_limit=32, timeout_ms=30000,
            proxy=None, state_parent=str(self.root), host_script=str(self.script))

    def exited(self, expected):
        stdout, stderr = self.process.communicate(timeout=4)
        self.assertEqual(self.process.returncode, expected)
        self.assertEqual(stdout, b'')
        self.assertEqual(stderr, b'')

    def test_unknown_plan_field_exits_before_ready(self):
        send(self.parent, self.plan() | {'unexpected': True})
        self.exited(2)
        self.assertEqual(list(self.root.glob('observed-*')), [])

    def test_oversized_frame_is_rejected_without_body(self):
        self.parent.sendall(struct.pack('!I', 98305))
        self.exited(2)

    def test_truncated_frame_is_not_a_valid_plan(self):
        self.parent.sendall(struct.pack('!I', 3) + b'{}')
        self.parent.shutdown(socket.SHUT_WR)
        self.exited(2)

    def test_wrong_start_nonce_never_loads_or_runs_the_account(self):
        send(self.parent, self.plan())
        self.assertEqual(receive(self.parent), {'command': 'ready', 'nonce': 'a' * 32})
        send(self.parent, dict(command='start', nonce='wrong',
                               session_cookie='session=private-fixture'))
        done = receive(self.parent)
        self.assertEqual(done, dict(command='done', nonce='a' * 32,
                                    local_pair_success=False, failure='Admission'))
        self.exited(1)
        # Only the owned profile marker may exist. No browser databases or logs.
        files = [p for p in self.root.rglob('*') if p.is_file()]
        self.assertEqual(sorted(p.name for p in files), ['.observer-owner', 'observed_sidecar.py'])
        self.assertNotIn('private-fixture', json.dumps(done))


if __name__ == '__main__':
    BINARY = Path(sys.argv.pop(1)).resolve(strict=True)
    unittest.main(verbosity=2)
