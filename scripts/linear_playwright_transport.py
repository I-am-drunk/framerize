"""Bound copying and release delivered messages in the pinned Python transport."""
import asyncio
import hashlib
import inspect
from pathlib import Path

SOURCE_SHA256 = "edac83a94c76c0f7cda78b58beaed30432fd43f611bb65480e52e160da54425b"
RUN_SHA256 = "366cfa477e221e17116cc9d2da7eb24620e7bcbd5f59a081c91fcaab67126d4d"


async def run_linear(self):
    assert self._proc.stdout
    assert self._proc.stdin
    try:
        while not self._stopped:
            try:
                header = await self._proc.stdout.readexactly(4)
                if self._stopped:
                    break
                remaining = int.from_bytes(header, byteorder="little", signed=False)
                parts = []
                while remaining:
                    size = min(remaining, 32768)
                    part = await self._proc.stdout.readexactly(size)
                    if self._stopped:
                        break
                    remaining -= size
                    parts.append(part)
                    del part
                if self._stopped:
                    break
                wire = parts[0] if len(parts) == 1 else b"".join(parts)
                parts.clear()
                del parts, header
                message = self.deserialize_message(wire)
                del wire
                self.on_message(message)
                del message
            except asyncio.IncompleteReadError:
                if not self._stopped:
                    self.on_error_future.set_exception(
                        Exception("Connection closed while reading from the driver"))
                break
            await asyncio.sleep(0)
        await self._proc.communicate()
    finally:
        if not self._stopped_future.done():
            self._stopped_future.set_result(None)


def install():
    try:
        from playwright._impl import _transport
        current = _transport.PipeTransport.run
    except (ImportError, AttributeError):
        return "unsupported"
    if current is run_linear:
        return "already_applied"
    try:
        source = Path(_transport.__file__).read_bytes()
        method = inspect.getsource(current).encode()
    except (OSError, TypeError, AttributeError):
        return "unsupported"
    if (hashlib.sha256(source).hexdigest() != SOURCE_SHA256
            or hashlib.sha256(method).hexdigest() != RUN_SHA256
            or current.__module__ != _transport.__name__):
        return "unsupported"
    _transport.PipeTransport.run = run_linear
    return "applied"
