"""Explicit private child entry point. The caller owns its outer process bound."""
import argparse
import asyncio
from contextlib import redirect_stderr, redirect_stdout
import math
import os
from pathlib import Path
import stat
import sys


class PipeProbe:
    def __init__(self, io):
        self.io = io
        self.used = self.closed = self.failed = False
        self.pending = None

    def current(self):
        return (not self.closed and not self.failed and not self.io.reader.at_eof()
                and not self.io.writer.is_closing())

    async def _exchange(self, url, milliseconds):
        from control_egress import _address
        from control_protocol import fields, refuse
        await self.io.write(dict(op="egress_probe", url=url, budget_ms=milliseconds))
        reply = await self.io.read()
        fields(reply, "version op url ip")
        if reply["op"] != "egress_result":
            refuse()
        result = {key: reply[key] for key in ("version", "url", "ip")}
        result["ip"] = _address(result, url)
        return result

    async def observe(self, url, milliseconds, stopped):
        from control_protocol import refuse
        if (self.used or not self.current() or stopped.is_set()
                or type(milliseconds) is not int or not 1 <= milliseconds <= 30000):
            refuse()
        self.used = True
        self.pending = asyncio.create_task(self._exchange(url, milliseconds))
        cancelled = False
        try:
            while not self.pending.done():
                try:
                    await asyncio.shield(self.pending)
                except asyncio.CancelledError:
                    cancelled = self.failed = True
                    stopped.set()
                except Exception:
                    break
            if cancelled:
                if not self.pending.cancelled():
                    self.pending.exception()
                raise asyncio.CancelledError()
            return self.pending.result()
        except BaseException:
            self.failed = True
            raise


def validate_settings(endpoint, revision, preparation, probe, witness):
    from control_egress import _probe_url
    from urllib.parse import urlsplit
    if type(revision) is not int or not 0 <= revision < 2 ** 64:
        raise ValueError("invalid browser control launcher settings")
    for value, limit in [(preparation, 180), (probe, 30), (witness, 180)]:
        if type(value) not in (int, float) or not math.isfinite(value) or not 0 < value <= limit:
            raise ValueError("invalid browser control launcher settings")
    generated = _probe_url(endpoint)
    separator = "&" if urlsplit(endpoint).query else ""
    if "?" not in endpoint:
        separator = "?"
    if "#" in endpoint or generated.rsplit("framerize_probe=", 1)[0] != endpoint + separator:
        raise ValueError("invalid browser control launcher settings")


async def run_child(io, endpoint, revision, preparation, probe, witness, *, host_factory=None):
    from control_child import PreparedControlChild
    from control_egress import EgressAttester, NativeRouteProbe
    validate_settings(endpoint, revision, preparation, probe, witness)
    native = PipeProbe(io)

    async def attest(host, identity, expected, proxy, stopped, deadline):
        route = NativeRouteProbe(identity.account, expected.user, proxy,
                                 native.observe, native.current)
        return await EgressAttester(endpoint, route, witness, probe)(
            host, identity, expected, proxy, stopped, deadline)

    options = dict(proxy_revision=revision, prepare_seconds=preparation)
    if host_factory is not None:
        options["host_factory"] = host_factory
    child = PreparedControlChild(attest, **options)
    try:
        return await child.run_frames(io)
    finally:
        native.closed = True


def private_directory(raw):
    path = Path(raw)
    info = path.lstat()
    if (not path.is_absolute() or path.resolve() != path or not stat.S_ISDIR(info.st_mode)
            or info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o700
            or next(path.iterdir(), None) is not None):
        raise ValueError("empty private browser control directory required")
    return path


def arguments(argv):
    parser = argparse.ArgumentParser(description="Explicit private browser credential child")
    parser.add_argument("--egress-endpoint", required=True)
    parser.add_argument("--state-directory", required=True)
    parser.add_argument("--proxy-revision", required=True, type=int)
    parser.add_argument("--prepare-seconds", required=True, type=float)
    parser.add_argument("--probe-seconds", required=True, type=float)
    parser.add_argument("--witness-seconds", required=True, type=float)
    return parser.parse_args(argv)


async def run_stdio(args):
    from control_protocol import Frames, open_stdio
    reader, writer, transport = await open_stdio()
    try:
        with open(os.devnull, "w") as quiet, redirect_stdout(quiet), redirect_stderr(quiet):
            return await run_child(Frames(reader, writer), args.egress_endpoint,
                args.proxy_revision, args.prepare_seconds, args.probe_seconds, args.witness_seconds)
    finally:
        transport.close()
        writer.close()
        closing = asyncio.create_task(writer.wait_closed())
        cancelled = False
        while not closing.done():
            try:
                await asyncio.shield(closing)
            except asyncio.CancelledError:
                cancelled = True
            except Exception:
                break
        closing.result()
        if cancelled:
            raise asyncio.CancelledError()


def main(argv=None):
    args = arguments(argv)
    try:
        directory = private_directory(args.state_directory)
        os.environ["FRAMERIZE_STATE"] = str(directory)
        sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "runtime"))
        return 0 if asyncio.run(run_stdio(args)) else 1
    except Exception:
        print("browser credential control failed", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
