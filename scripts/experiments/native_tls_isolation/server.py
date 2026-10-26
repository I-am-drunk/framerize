#!/usr/bin/env python3
"""Bounded synthetic HTTPS fixture. Run with --tls-version 1.2 or 1.3.

Requires Python 3.8+ with TLS support and openssl req supporting -addext.
Stdout emits only one flushed JSON line containing port and certificate_pem.
The certificate covers IP 127.0.0.1; temporary certificate/key files are removed
before listening. Keep stdin open, then send stop followed by a newline or EOF.
POST JSON to /ai/v3/responses/ or another /ai/ path using HTTP/1.1 and an
explicit Content-Length. Missing Cookie/Authorization values echo as "".
Duplicate headers/JSON keys, chunked encoding and Expect are rejected.
fixture_close=true in a JSON object closes after replying; otherwise the
connection stays open within the resource limits below. Counts start at one.
Errors expose only their type on stderr, never request data or key material.
"""

import argparse
import json
import os
from pathlib import Path
import re
import select
import socket
import ssl
import subprocess
import sys
import tempfile
import threading
import time

HEADER_LIMIT = 16 * 1024
BODY_LIMIT = 64 * 1024
MAX_ACTIVE = 16
MAX_CONNECTIONS = 128
MAX_REQUESTS = 512
MAX_PER_CONNECTION = 64
OPERATION_SECONDS = 10.0
LIFETIME_SECONDS = 120.0
CLEANUP_SECONDS = 5.0
CONTROL_LIMIT = 1024


class ProtocolError(ValueError):
    pass


class LimitError(ValueError):
    pass


class CleanupError(RuntimeError):
    pass


def report(error):
    print("native-tls-fixture: " + type(error).__name__, file=sys.stderr, flush=True)


def set_deadline(sock, deadline):
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        raise TimeoutError
    sock.settimeout(remaining)


def shutdown_socket(sock):
    try:
        # Wake a handler without mutating SSLSocket's active SSL object.
        socket.socket.shutdown(sock, socket.SHUT_RDWR)
    except OSError:
        pass


def close_connection(sock):
    try:
        # Send close_notify, but never wait indefinitely for the peer's reply.
        sock.settimeout(0.25)
        plain = sock.unwrap()
        plain.close()
    except (OSError, ValueError):
        pass
    finally:
        try:
            sock.close()
        except OSError as error:
            report(error)


def reject_constant(_value):
    raise ProtocolError


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ProtocolError
        result[key] = value
    return result


def parse_headers(block):
    lines = block.split(b"\r\n")
    parts = lines[0].split(b" ")
    if len(parts) != 3 or parts[0] != b"POST" or parts[2] != b"HTTP/1.1":
        raise ProtocolError
    if not parts[1].startswith(b"/ai/") or any(b < 33 or b > 126 for b in parts[1]):
        raise ProtocolError
    headers = {}
    for line in lines[1:]:
        name, colon, value = line.partition(b":")
        if not colon or not re.fullmatch(rb"[!#$%&'*+.^_`|~0-9A-Za-z-]+", name):
            raise ProtocolError
        if any(b == 127 or (b < 32 and b != 9) for b in value):
            raise ProtocolError
        name = name.decode("ascii").lower()
        if name in headers:
            raise ProtocolError
        headers[name] = value.strip(b" \t").decode("latin-1")
    if not headers.get("host") or any(k in headers for k in ("transfer-encoding", "expect", "upgrade")):
        raise ProtocolError
    return headers


def read_request(sock, pending, deadline):
    while b"\r\n\r\n" not in pending:
        if len(pending) >= HEADER_LIMIT:
            raise LimitError
        set_deadline(sock, deadline)
        chunk = sock.recv(min(4096, HEADER_LIMIT - len(pending)))
        if not chunk:
            if pending:
                raise ProtocolError
            return None
        pending.extend(chunk)
    end = pending.index(b"\r\n\r\n") + 4
    if end > HEADER_LIMIT:
        raise LimitError
    headers = parse_headers(bytes(pending[:end - 4]))
    del pending[:end]
    length = headers.get("content-length", "")
    if not re.fullmatch(r"[0-9]{1,5}", length):
        raise ProtocolError
    length = int(length)
    if length > BODY_LIMIT:
        raise LimitError
    while len(pending) < length:
        set_deadline(sock, deadline)
        chunk = sock.recv(min(4096, length - len(pending)))
        if not chunk:
            raise ProtocolError
        pending.extend(chunk)
    raw_body = bytes(pending[:length])
    del pending[:length]
    body = json.loads(raw_body.decode("utf-8"), parse_constant=reject_constant,
                      object_pairs_hook=unique_object)
    return headers, body


def send_response(sock, result, close, deadline):
    body = json.dumps(result, ensure_ascii=True, allow_nan=False,
                      separators=(",", ":")).encode("ascii")
    if len(body) > 512 * 1024:
        raise LimitError
    header = ("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n"
              f"Content-Length: {len(body)}\r\n"
              f"Connection: {'close' if close else 'keep-alive'}\r\n\r\n")
    set_deadline(sock, deadline)
    sock.sendall(header.encode("ascii") + body)


class Fixture:
    def __init__(self, context):
        self.context = context
        self.stopping = threading.Event()
        self.lock = threading.Lock()
        self.sockets = {}
        self.workers = []
        self.connections = 0
        self.requests = 0
        self.listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        try:
            self.listener.bind(("127.0.0.1", 0))
            self.listener.listen(MAX_ACTIVE)
            self.listener.setblocking(False)
        except BaseException:
            self.listener.close()
            raise

    def handle(self, number, sock):
        try:
            sock.settimeout(OPERATION_SECONDS)
            sock.do_handshake()
            resumed, version = sock.session_reused, sock.version()
            pending = bytearray()
            for count in range(1, MAX_PER_CONNECTION + 1):
                if self.stopping.is_set():
                    break
                deadline = time.monotonic() + OPERATION_SECONDS
                request = read_request(sock, pending, deadline)
                if request is None:
                    break
                headers, body = request
                with self.lock:
                    if self.requests >= MAX_REQUESTS:
                        raise LimitError
                    self.requests += 1
                close = (isinstance(body, dict) and body.get("fixture_close") is True)
                close = close or count == MAX_PER_CONNECTION
                tokens = headers.get("connection", "").lower().split(",")
                close = close or any(token.strip() == "close" for token in tokens)
                result = {"connection": number, "resumed": resumed, "tls_version": version,
                          "cookie": headers.get("cookie", ""),
                          "authorization": headers.get("authorization", ""),
                          "request_body": body, "requests_on_connection": count}
                send_response(sock, result, close, deadline)
                if close:
                    break
        except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError,
                ssl.SSLEOFError, ssl.SSLZeroReturnError):
            pass
        except Exception as error:
            if not self.stopping.is_set():
                report(error)
        finally:
            close_connection(sock)
            with self.lock:
                self.sockets.pop(number, None)

    def accept(self):
        raw, _address = self.listener.accept()
        self.connections += 1
        if self.connections > MAX_CONNECTIONS:
            raw.close()
            raise LimitError
        with self.lock:
            full = len(self.sockets) >= MAX_ACTIVE
        if full:
            raw.close()
            report(LimitError())
            return
        try:
            sock = self.context.wrap_socket(raw, server_side=True,
                                            do_handshake_on_connect=False)
        except BaseException:
            raw.close()
            raise
        number = self.connections
        worker = threading.Thread(target=self.handle, args=(number, sock), daemon=True)
        with self.lock:
            self.sockets[number] = sock
        try:
            worker.start()
        except BaseException:
            sock.close()
            with self.lock:
                self.sockets.pop(number, None)
            raise
        self.workers.append(worker)

    def run(self):
        deadline = time.monotonic() + LIFETIME_SECONDS
        control = bytearray()
        stdin = sys.stdin.fileno()
        while not self.stopping.is_set():
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError
            ready, _, _ = select.select([stdin, self.listener], [], [], min(0.2, remaining))
            if stdin in ready:
                chunk = os.read(stdin, CONTROL_LIMIT + 1 - len(control))
                if not chunk:
                    return
                control.extend(chunk)
                if len(control) > CONTROL_LIMIT:
                    raise LimitError
                while b"\n" in control:
                    line, _, rest = control.partition(b"\n")
                    control = bytearray(rest)
                    if line.rstrip(b"\r") == b"stop":
                        return
                    if line:
                        raise ProtocolError
            if self.listener in ready:
                try:
                    self.accept()
                except BlockingIOError:
                    pass

    def close(self):
        self.stopping.set()
        self.listener.close()
        with self.lock:
            sockets = list(self.sockets.values())
        for sock in sockets:
            shutdown_socket(sock)
        deadline = time.monotonic() + CLEANUP_SECONDS
        for worker in self.workers:
            worker.join(max(0.0, deadline - time.monotonic()))
        for sock in sockets:
            sock.close()
        if any(worker.is_alive() for worker in self.workers):
            raise CleanupError


def make_context(version):
    selected = {"1.2": ssl.TLSVersion.TLSv1_2, "1.3": ssl.TLSVersion.TLSv1_3}[version]
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = selected
    context.maximum_version = selected
    context.options &= ~ssl.OP_NO_TICKET
    if version == "1.3":
        if not ssl.HAS_TLSv1_3:
            raise RuntimeError
        context.num_tickets = 2
    context.set_alpn_protocols(["http/1.1"])
    with tempfile.TemporaryDirectory(prefix="native-tls-isolation-") as directory:
        key = Path(directory) / "key.pem"
        cert = Path(directory) / "cert.pem"
        subprocess.run(
            ["openssl", "req", "-x509", "-newkey", "rsa:2048", "-sha256", "-nodes",
             "-days", "1", "-subj", "/CN=127.0.0.1", "-keyout", str(key),
             "-out", str(cert), "-addext", "subjectAltName=IP:127.0.0.1",
             "-addext", "basicConstraints=critical,CA:FALSE",
             "-addext", "keyUsage=critical,digitalSignature,keyEncipherment",
             "-addext", "extendedKeyUsage=serverAuth"],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL, timeout=15, check=True,
        )
        key.chmod(0o600)
        context.load_cert_chain(cert, key)
        certificate_pem = cert.read_text(encoding="ascii")
    return context, certificate_pem


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tls-version", choices=("1.2", "1.3"), required=True)
    args = parser.parse_args()
    context, certificate_pem = make_context(args.tls_version)
    fixture = Fixture(context)
    try:
        print(json.dumps({"port": fixture.listener.getsockname()[1],
                          "certificate_pem": certificate_pem}), flush=True)
        fixture.run()
    finally:
        fixture.close()


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        pass
    except Exception as error:
        report(error)
        sys.exit(1)
