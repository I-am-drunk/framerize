"""Bounded HTTPS fixture. Only synthetic state and loopback requests exist."""
import asyncio
import json
import ssl
import subprocess
from pathlib import Path


class Server:
    def __init__(self):
        self.release = asyncio.Event()
        self.cancel_started = asyncio.Event()
        self.cancel_closed = asyncio.Event()
        self.tasks, self.writers = set(), set()
        self.connections, self.records, self.errors = 0, [], []

    async def start(self, directory):
        cert, key = (Path(directory) / name for name in ("cert.pem", "key.pem"))
        subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048",
                        "-nodes", "-days", "1", "-subj", "/CN=localhost",
                        "-addext", "subjectAltName=IP:127.0.0.1",
                        "-keyout", str(key), "-out", str(cert)], check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=5)
        key.chmod(0o600)
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(cert, key)
        context.set_alpn_protocols(["http/1.1"])
        self.server = await asyncio.start_server(self.handle, "127.0.0.1", 0,
                                                ssl=context, limit=16384)
        self.origin = "https://127.0.0.1:" + str(self.server.sockets[0].getsockname()[1])

    async def handle(self, reader, writer):
        task = asyncio.current_task()
        self.tasks.add(task)
        self.writers.add(writer)
        self.connections += 1
        connection = self.connections
        try:
            while True:
                head = await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), 15)
                lines = head.decode("ascii").split("\r\n")
                method, path, _ = lines[0].split()
                if method != "GET" or path not in ("/", "/echo", "/hold", "/cancel", "/favicon.ico"):
                    raise ValueError("unexpected fixture request: " + method + " " + path)
                headers = {name.lower(): value.strip() for name, value in
                           (line.split(":", 1) for line in lines[1:] if line)}
                record = dict(connection=connection, cookie=headers.get("cookie", ""))
                self.records.append(record)
                if path in ("/hold", "/cancel"):
                    await self.stream(reader, writer, path)
                    break
                body = b"<!doctype html><title>isolation fixture</title><link rel=icon href=data:,>" if path == "/" else json.dumps(record).encode()
                if path == "/favicon.ico":
                    body = b""
                await self.write(writer, body, "text/html" if path == "/" else "application/json")
        except (asyncio.IncompleteReadError, ConnectionError, asyncio.CancelledError):
            pass
        except Exception as error:
            self.errors.append(type(error).__name__ + ": " + str(error))
        finally:
            writer.close()
            try:
                await asyncio.wait_for(writer.wait_closed(), 2)
            except (ConnectionError, TimeoutError):
                pass
            self.writers.discard(writer)
            self.tasks.discard(task)

    async def write(self, writer, body, content_type):
        head = ("HTTP/1.1 200 OK\r\nContent-Length: " + str(len(body)) +
                "\r\nContent-Type: " + content_type + "\r\nCache-Control: no-store\r\n\r\n")
        writer.write(head.encode() + body)
        await writer.drain()

    async def stream(self, reader, writer, path):
        writer.write(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n"
                     b"Content-Type: text/plain\r\nCache-Control: no-store\r\n\r\nfirst")
        await writer.drain()
        if path == "/hold":
            await self.release.wait()
            writer.write(b"-last")
            await writer.drain()
        else:
            self.cancel_started.set()
            try:
                remaining = await asyncio.wait_for(reader.read(1), 10)
            except ConnectionError:
                remaining = b""
            if remaining != b"":
                raise ValueError("unexpected data while response held")
            self.cancel_closed.set()

    async def close(self):
        self.release.set()
        self.server.close()
        await self.server.wait_closed()
        for writer in list(self.writers):
            writer.close()
        pending = list(self.tasks)
        for task in pending:
            task.cancel()
        if pending:
            await asyncio.wait_for(asyncio.gather(*pending, return_exceptions=True), 3)
        if self.tasks or self.writers:
            raise RuntimeError("fixture handlers did not join")
