"""Private one-use transport for an already prepared ControlHost.

The launcher supplies the host and an actual-egress validity predicate.
Neither this module nor its protocol prepares a page or discovers credentials.
"""
import asyncio
import json
import time
import uuid

from control_mint import ExpectedContext, ObservationRefused
from control_owner import Consumed, ControlIdentity, _Binding

MAX_FRAME = 128 * 1024


def refuse():
    raise ObservationRefused()


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            refuse()
        result[key] = value
    return result


def fields(value, names):
    if type(value) is not dict or set(value) != set(names.split()):
        refuse()


class Frames:
    def __init__(self, reader, writer):
        self.reader, self.writer, self.pending = reader, writer, bytearray()

    async def read(self):
        while b"\n" not in self.pending:
            if len(self.pending) > MAX_FRAME:
                refuse()
            part = await self.reader.read(min(4096, MAX_FRAME + 1 - len(self.pending)))
            if not part:
                refuse()
            self.pending.extend(part)
        end = self.pending.index(b"\n")
        if end > MAX_FRAME:
            refuse()
        raw = bytes(self.pending[:end])
        del self.pending[:end + 1]
        value = json.loads(raw.decode("utf-8"), object_pairs_hook=unique_object,
                          parse_constant=lambda _: refuse())
        if type(value) is not dict or type(value.get("version")) is not int:
            refuse()
        if value["version"] != 1:
            refuse()
        return value

    async def write(self, value):
        data = json.dumps(dict(version=1, **value), ensure_ascii=False,
                          allow_nan=False, separators=(",", ":")).encode()
        if len(data) > MAX_FRAME:
            refuse()
        self.writer.write(data + b"\n")
        await self.writer.drain()


class ControlProtocol:
    def __init__(self, host, identity, expected, proxy, egress_current):
        if (type(identity) is not ControlIdentity
                or type(expected) is not ExpectedContext or not callable(egress_current)):
            refuse()
        self.host, self.identity, self.expected = host, identity, expected
        self.proxy, self.egress_current = proxy, egress_current
        self.binding = dict(account=identity.account, user=expected.user,
            session=expected.session, device=expected.device, page_url=expected.page_url,
            wrapper_url=expected.wrapper_url, post_path=expected.post_path, proxy=proxy)
        self.grant = self.control = self.complete = self.minted = None
        self.known = False
        self.reason = "unconfirmed"

    def current(self):
        if self.egress_current() is not True:
            refuse()
        if self.host.proxies.get(self.identity.account) != self.proxy:
            refuse()

    async def prepare(self, io, request=None):
        if request is None:
            request = await io.read()
        fields(request, "version op binding")
        if (type(request["version"]) is not int or request["version"] != 1
                or request["op"] != "prepare" or request["binding"] != self.binding):
            refuse()
        _Binding(self.host, self.identity, self.expected)
        self.current()
        await io.write(dict(op="ready", host_generation=self.identity.host_generation,
            incarnation=self.identity.incarnation, proxy_revision=self.identity.proxy_revision))

    def admit(self, request):
        fields(request, "version op grant budget_ms request_ms")
        if request["op"] != "mint":
            refuse()
        grant = request["grant"]
        fields(grant, "account native_generation host_generation lease device authority")
        generation, lease = grant["host_generation"], grant["lease"]
        if (type(generation) is not str or len(generation) != 32
                or any(c not in "0123456789abcdefABCDEF" for c in generation)
                or type(lease) is not str or str(uuid.UUID(lease)) != lease
                or uuid.UUID(lease).int == 0 or grant["account"] != self.identity.account
                or grant["device"] != self.expected.device
                or grant["authority"] not in ("ledger", "observation")
                or type(grant["native_generation"]) is not int
                or not 0 < grant["native_generation"] < 2 ** 64):
            refuse()
        budget, duration = request["budget_ms"], request["request_ms"]
        if (type(budget) is not int or type(duration) is not int
                or not 1 <= budget <= 16000 or not budget <= duration <= 180000):
            refuse()
        self.current()
        self.grant = grant
        return budget / 1000, duration / 1000

    async def consume(self, minted, cancelled, deadline):
        self.current()
        self.minted.set_result(minted.token)
        return await self.complete

    def completion(self, request, delivered):
        op = request.get("op")
        fields(request, "version op grant joined risk_refused" if op == "finish"
               else "version op grant")
        grant = request["grant"]
        if (type(grant) is not dict or grant != self.grant
                or any(type(grant[k]) is not type(v) for k, v in self.grant.items())):
            refuse()
        if op == "cancel":
            self.known, self.reason = True, "cancelled"
        elif (op == "finish" and delivered and type(request["joined"]) is bool
                and type(request["risk_refused"]) is bool):
            self.current()
            self.known, self.reason = request["joined"], "finished"
        else:
            refuse()

    async def mint(self, io, request):
        budget, duration = self.admit(request)
        loop = asyncio.get_running_loop()
        self.complete, self.minted = loop.create_future(), loop.create_future()
        deadline = time.monotonic() + duration
        self.control = asyncio.create_task(self.host.run_control(
            self.identity, self.expected, self.consume, deadline))
        incoming = asyncio.create_task(io.read())
        mint_clock = asyncio.create_task(asyncio.sleep(budget))
        end_clock = asyncio.create_task(asyncio.sleep(duration))
        tasks = [incoming, mint_clock, end_clock]
        try:
            await self.wait_mint(io, incoming, mint_clock, end_clock)
        finally:
            for task in tasks:
                task.cancel()
            await asyncio.gather(*tasks, return_exceptions=True)

    async def wait_mint(self, io, incoming, mint_clock, end_clock):
        done, _ = await asyncio.wait([incoming, mint_clock, self.minted, self.control],
                                     return_when=asyncio.FIRST_COMPLETED)
        if incoming in done:
            self.completion(incoming.result(), False)
            return
        if mint_clock in done or self.control in done:
            self.known = True
            self.reason = "deadline" if mint_clock in done else "rejected"
            return
        await io.write(dict(op="minted", grant=self.grant, token=self.minted.result()))
        done, _ = await asyncio.wait([incoming, end_clock, self.control],
                                     return_when=asyncio.FIRST_COMPLETED)
        if end_clock in done or self.control in done:
            self.reason = "unconfirmed"
            return
        self.completion(incoming.result(), True)

    async def settle(self):
        if self.reason != "finished" or not self.known:
            self.host._control_fenced = True
            self.host._control_cancelled.set()
        if self.complete is not None and not self.complete.done():
            self.complete.set_result(Consumed(None, self.known))
        if self.control is not None:
            result = await asyncio.gather(asyncio.shield(self.control), return_exceptions=True)
            if self.reason == "finished" and isinstance(result[0], BaseException):
                self.known = False
        try:
            await self.host.close()
            return self.known
        except Exception:
            return False

    async def run(self, reader, writer):
        return await self.run_frames(Frames(reader, writer))

    async def run_frames(self, io, first_request=None):
        writer = io.writer
        cancelled = False
        try:
            await self.prepare(io, first_request)
            request = await io.read()
            if request == {"version": 1, "op": "shutdown"}:
                self.known, self.reason = True, "finished"
            else:
                await self.mint(io, request)
        except (Exception, asyncio.CancelledError) as error:
            cancelled = isinstance(error, asyncio.CancelledError)
            self.known, self.reason = False, "unconfirmed"
        finally:
            finalizer = asyncio.create_task(self.finish_io(io, writer))
            while True:
                try:
                    confirmed = await asyncio.shield(finalizer)
                    break
                except asyncio.CancelledError:
                    if finalizer.cancelled():
                        raise
                    cancelled = True
                    self.known, self.reason = False, "unconfirmed"
                    self.host._control_fenced = True
                    self.host._control_cancelled.set()
        if cancelled:
            raise asyncio.CancelledError()
        return confirmed


    async def finish_io(self, io, writer):
        confirmed = await self.settle()
        try:
            await io.write(dict(op="closed", grant=self.grant,
                cleanup_confirmed=confirmed, reason=self.reason))
        except Exception:
            confirmed = False
        finally:
            writer.close()
            try:
                await writer.wait_closed()
            except Exception:
                confirmed = False
        return confirmed


class _PipeWriter(asyncio.streams.FlowControlMixin):
    def __init__(self, loop):
        super().__init__(loop=loop)
        self.closed = loop.create_future()

    def connection_lost(self, error):
        super().connection_lost(error)
        if not self.closed.done():
            self.closed.set_result(None)

    def _get_close_waiter(self, stream):
        return self.closed


async def open_stdio():
    """The launcher must close the returned input transport in its finally."""
    import sys
    loop = asyncio.get_running_loop()
    reader = asyncio.StreamReader(limit=MAX_FRAME + 1)
    protocol = asyncio.StreamReaderProtocol(reader)
    input_transport, _ = await loop.connect_read_pipe(lambda: protocol, sys.stdin.buffer)
    try:
        output, writer_protocol = await loop.connect_write_pipe(
            lambda: _PipeWriter(loop), sys.stdout.buffer)
    except BaseException:
        input_transport.close()
        raise
    return reader, asyncio.StreamWriter(output, writer_protocol, None, loop), input_transport
