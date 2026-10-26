"""Local HTTPS bodies for real-browser response ownership checks."""
import asyncio
import json
from server import Server


class ResponseServer(Server):
    def __init__(self, limit):
        super().__init__()
        self.limit = limit
        self.started = {name: asyncio.Event() for name in ('oversize', 'cancel')}
        self.disconnected = {name: asyncio.Event() for name in self.started}
        self.paths = []

    async def handle(self, reader, writer):
        task = asyncio.current_task()
        self.tasks.add(task)
        self.writers.add(writer)
        try:
            head = await asyncio.wait_for(reader.readuntil(b'\r\n\r\n'), 10)
            lines = head.decode('ascii').split('\r\n')
            method, path, _ = lines[0].split()
            headers = {k.lower(): v.strip() for k, v in
                       (line.split(':', 1) for line in lines[1:] if line)}
            if method == 'GET' and path in ('/', '/favicon.ico'):
                await self.write(writer, b'<title>local fixture</title>', 'text/html')
                return
            length = int(headers.get('content-length', '-1'))
            if method != 'POST' or not 0 <= length <= 1024:
                raise ValueError('unexpected request')
            body = json.loads(await asyncio.wait_for(reader.readexactly(length), 5))
            if (headers.get('authorization') != 'Bearer synthetic-bearer'
                    or body != {'castleToken': 'synthetic-token'}):
                raise ValueError('unexpected synthetic credentials or body')
            if path not in ('/exact', '/oversize', '/cancel'):
                raise ValueError('unexpected fixture path')
            self.paths.append(path)
            await self.body(reader, writer, path[1:])
        except (asyncio.IncompleteReadError, ConnectionError):
            pass
        except Exception as error:
            self.errors.append(type(error).__name__)
        finally:
            writer.close()
            try:
                await asyncio.wait_for(writer.wait_closed(), 2)
            except (ConnectionError, TimeoutError):
                pass
            self.writers.discard(writer)
            self.tasks.discard(task)

    async def body(self, reader, writer, name):
        declared = self.limit if name == 'exact' else self.limit + 65536
        head = ('HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n'
                'X-Framer-AI-Usage: synthetic\r\nConnection: close\r\n'
                f'Content-Length: {declared}\r\n\r\n')
        writer.write(head.encode('ascii'))
        remaining = self.limit if name == 'exact' else self.limit + 1
        if name == 'cancel':
            remaining = 1
        try:
            while remaining:
                count = min(32768, remaining)
                writer.write(b'a' * count)
                remaining -= count
                await asyncio.wait_for(writer.drain(), 5)
            if name == 'exact':
                return
            self.started[name].set()
            if await asyncio.wait_for(reader.read(1), 10) != b'':
                raise ValueError('unexpected data while body held')
        except ConnectionError:
            if name == 'exact':
                raise
        self.disconnected[name].set()
