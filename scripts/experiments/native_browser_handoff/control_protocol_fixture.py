"""Inert process fixture. Never launches a browser or accesses a real account."""
import asyncio
import json
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path[:0] = [str(ROOT / "runtime"), str(ROOT / "runtime/tests")]
import test_control_owner as fixtures
from control_mint import ExpectedContext
from control_owner import ControlIdentity
from control_protocol import ControlProtocol, Frames, open_stdio


class PrefixReader:
    def __init__(self, prefix, reader):
        self.prefix, self.reader = bytearray(prefix), reader

    async def read(self, count):
        if self.prefix:
            part = bytes(self.prefix[:count])
            del self.prefix[:count]
            return part
        return await self.reader.read(count)


async def gate(folder, name):
    (folder / (name + "-entered")).touch()
    while not (folder / (name + "-release")).exists():
        await asyncio.sleep(0.01)


class FaultWriter:
    def __init__(self, writer, mode, folder):
        self.writer, self.mode, self.folder = writer, mode, folder
        self.tail = None

    def write(self, data):
        value = json.loads(data)
        if value["op"] == "minted" and self.mode == "wrong-lease":
            value["grant"]["lease"] = "00000000-0000-4000-8000-000000000009"
        if value["op"] == "minted" and self.mode == "oversized":
            self.writer.write(b"x" * (128 * 1024 + 2))
            return
        if value["op"] == "minted" and self.mode == "duplicate":
            data = b'{"version":1,' + json.dumps(value).encode()[1:] + b"\n"
        else:
            data = json.dumps(value).encode() + b"\n"
        if value["op"] == "minted" and self.mode == "partial-mint":
            split = len(data) // 2
            self.tail, data = data[split:], data[:split]
        self.writer.write(data)
        if value["op"] == "closed" and self.mode == "extra-frame":
            self.writer.write(b'{"unexpected":true}\n')

    async def drain(self):
        if self.tail is not None:
            await gate(self.folder, "partial")
            self.writer.write(self.tail)
            self.tail = None
        await self.writer.drain()

    def close(self):
        self.writer.close()

    async def wait_closed(self):
        await self.writer.wait_closed()


async def main():
    mode, folder = sys.argv[1], Path(sys.argv[2])
    (folder / "pid").write_text(str(os.getpid()))
    reader, writer, input_transport = await open_stdio()
    request = await Frames(reader, writer).read()
    binding = request["binding"]
    f = fixtures.OwnerTests("test_success_joins_one_consumer_and_rejects_control_reuse")
    await f.asyncSetUp()
    account = binding["account"]
    f.host.accounts = {account: f.acc}
    f.host.lifecycles = {account: f.life}
    expected = ExpectedContext(*(binding[k] for k in (
        "user", "device", "session", "page_url", "wrapper_url", "post_path")))
    f.acc.session_cookie = ("session", expected.session)
    f.page.url = expected.page_url
    device = next(iter(f.acc.dev_objs.values()))
    device.id = expected.device
    f.acc.dev_objs = {expected.device: device}
    if binding["proxy"] is not None:
        f.host.proxies[account] = binding["proxy"]
    async def cookies(url):
        return [dict(name="session", value=expected.session),
                dict(name="__cuid", value=expected.device)]
    f.page.context.cookies = cookies
    identity = ControlIdentity(account, f.host.generation, 1, 0)
    original_evaluate = f.page.evaluate
    async def evaluate(*args):
        if mode == "hold-mint":
            await gate(folder, "mint")
        return await original_evaluate(*args)
    f.page.evaluate = evaluate
    original_close = f.browser._impl_obj._channel.send
    async def close(*args):
        if mode == "hold-close":
            await gate(folder, "close")
        return await original_close(*args)
    f.browser._impl_obj._channel.send = close
    peer = ControlProtocol(f.host, identity, expected, binding["proxy"],
                           lambda: mode != "bad-egress")
    prefixed = PrefixReader(json.dumps(request).encode() + b"\n", reader)
    try:
        if mode == "hold-prepare":
            await gate(folder, "prepare")
        result = await peer.run(prefixed, FaultWriter(writer, mode, folder))
        (folder / "physically-closed").write_text(str(not f.browser.connected))
        if mode == "hold-exit":
            await gate(folder, "exit")
        return 0 if result and mode != "bad-exit" else 2
    finally:
        input_transport.close()
        await f.asyncTearDown()
        f.doCleanups()


if __name__ == "__main__":
    try:
        status = asyncio.run(main())
    except BaseException:
        status = 3
    sys.exit(status)
