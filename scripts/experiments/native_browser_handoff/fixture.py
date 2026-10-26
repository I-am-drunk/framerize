"""Local TLS peer and CONNECT proxy for unchanged production origin URLs."""
import asyncio
import json
from dataclasses import dataclass, field
from urllib.parse import urlsplit

from server import Server


@dataclass
class Reply:
    body: bytes
    status: int = 200
    held: bool = False
    started: asyncio.Event = field(default_factory=asyncio.Event)
    release: asyncio.Event = field(default_factory=asyncio.Event)
    closed: asyncio.Event = field(default_factory=asyncio.Event)


class HandoffServer(Server):
    def __init__(self):
        super().__init__()
        self.expected, self.replies, self.posts = {}, {}, {}
        self.documents = self.tunnels = self.denied = 0

    async def start(self, directory):
        await super().start(directory)
        self.proxy = await asyncio.start_server(self.connect, "127.0.0.1", 0, limit=16384)
        self.proxy_url = "http://127.0.0.1:" + str(self.proxy.sockets[0].getsockname()[1])

    async def connect(self, reader, writer):
        task, peer, pumps = asyncio.current_task(), None, []
        self.tasks.add(task)
        self.writers.add(writer)
        try:
            head = await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), 5)
            target = head.split(b"\r\n", 1)[0].split()
            if target not in ([b"CONNECT", b"framer.com:443", b"HTTP/1.1"],
                              [b"CONNECT", b"api.framer.com:443", b"HTTP/1.1"]):
                self.denied += 1
                writer.write(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
                await writer.drain()
                return
            self.tunnels += 1
            upstream, peer = await asyncio.open_connection("127.0.0.1", urlsplit(self.origin).port)
            writer.write(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            await writer.drain()
            async def pump(source, destination):
                while data := await source.read(32768):
                    destination.write(data)
                    await destination.drain()
            pumps = [asyncio.create_task(pump(reader, peer)),
                     asyncio.create_task(pump(upstream, writer))]
            await asyncio.wait(pumps, timeout=30, return_when=asyncio.FIRST_COMPLETED)
        except (ConnectionError, asyncio.IncompleteReadError, asyncio.CancelledError):
            pass
        except Exception as error:
            self.errors.append("proxy: " + type(error).__name__)
        finally:
            for pump in pumps:
                pump.cancel()
            await asyncio.gather(*pumps, return_exceptions=True)
            await self.close_writer(peer)
            await self.close_writer(writer)
            self.writers.discard(writer)
            self.tasks.discard(task)

    async def cors(self, writer, status, body, reader=None, reply=None):
        head = (f"HTTP/1.1 {status} Fixture\r\nContent-Length: {len(body)}\r\n"
                "Content-Type: text/event-stream\r\nConnection: close\r\n"
                "Access-Control-Allow-Origin: https://framer.com\r\n"
                "Access-Control-Allow-Credentials: true\r\n"
                "Access-Control-Allow-Methods: POST, OPTIONS\r\n"
                "Access-Control-Allow-Headers: authorization, content-type, x-requested-by\r\n"
                "Access-Control-Expose-Headers: x-framer-ai-usage\r\n"
                "X-Framer-AI-Usage: synthetic-usage\r\n")
        if reply is not None:
            head += "Set-Cookie: fixture_seen=1; Secure; SameSite=None; Path=/\r\n"
        head += "\r\n"
        writer.write(head.encode() + (body[:1] if reply and reply.held else body))
        await writer.drain()
        if reply is None:
            return
        reply.started.set()
        if reply.held:
            release = asyncio.create_task(reply.release.wait())
            disconnected = asyncio.create_task(reader.read(1))
            try:
                done, _ = await asyncio.wait((release, disconnected), timeout=20,
                                            return_when=asyncio.FIRST_COMPLETED)
                if release in done:
                    writer.write(body[1:])
                    await writer.drain()
            finally:
                release.cancel()
                disconnected.cancel()
                await asyncio.gather(release, disconnected, return_exceptions=True)

    @staticmethod
    async def close_writer(writer):
        if writer is not None:
            writer.close()
            try:
                await asyncio.wait_for(writer.wait_closed(), 2)
            except (ConnectionError, TimeoutError):
                pass

    async def close(self):
        self.proxy.close()
        await self.proxy.wait_closed()
        for reply in self.replies.values():
            reply.release.set()
        await super().close()

    async def handle(self, reader, writer):
        task, reply = asyncio.current_task(), None
        self.tasks.add(task)
        self.writers.add(writer)
        self.connections += 1
        connection = self.connections
        try:
            head = await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), 5)
            lines = head.decode("ascii").split("\r\n")
            method, path, _ = lines[0].split()
            headers = {k.lower(): v.strip() for k, v in
                       (line.split(":", 1) for line in lines[1:] if line)}
            if method == "GET" and path == "/login" and headers.get("host") == "framer.com":
                if "session=" in headers.get("cookie", ""):
                    raise ValueError("session cookie reached the document origin")
                self.documents += 1
                await self.write(writer, b"<title>local fixture</title><link rel=icon href=data:,>", "text/html")
                return
            if headers.get("host") != "api.framer.com" or path != "/ai/v3/responses/":
                raise ValueError("unexpected fixture route")
            if headers.get("origin") != "https://framer.com":
                raise ValueError("unexpected request origin")
            if method == "OPTIONS":
                await self.cors(writer, 204, b"")
                return
            length = int(headers.get("content-length", "-1"))
            if method != "POST" or not 0 <= length <= 4096:
                raise ValueError("unexpected fixture request")
            wire = await asyncio.wait_for(reader.readexactly(length), 5)
            case = json.loads(wire)["case"]
            expected = self.expected[case]
            cookies = dict(part.strip().split("=", 1) for part in headers.get("cookie", "").split(";"))
            if (case in self.posts or wire != expected["wire"].encode()
                    or headers.get("authorization") != "Bearer " + expected["bearer"]
                    or cookies != dict(part.strip().split("=", 1)
                                       for part in expected["cookie_header"].split(";"))
                    or headers.get("x-requested-by") != "Framer"):
                raise ValueError("duplicate send or crossed request identity")
            self.posts[case] = connection
            reply = self.replies[case]
            await self.cors(writer, reply.status, reply.body, reader, reply)
        except (ConnectionError, asyncio.IncompleteReadError, asyncio.CancelledError):
            pass
        except Exception as error:
            self.errors.append("peer: " + type(error).__name__)
        finally:
            await self.close_writer(writer)
            if reply is not None:
                reply.closed.set()
            self.writers.discard(writer)
            self.tasks.discard(task)
