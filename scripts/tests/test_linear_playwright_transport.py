"""Exercise real transport methods with synthetic byte streams only."""
import ast
import asyncio
import gc
import json
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import weakref

sys.path.insert(0, str(Path(__file__).parents[1]))
from linear_playwright_transport import run_linear, install


def original_run():
    source = Path(__file__).parents[2] / "reference/fixtures/playwright_transport_original.py"
    tree = ast.parse(source.read_text())
    cls = next(n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == "PipeTransport")
    method = next(n for n in cls.body if isinstance(n, ast.AsyncFunctionDef) and n.name == "run")
    namespace = {"asyncio": asyncio}
    exec(compile(ast.Module(body=[method], type_ignores=[]), str(source), "exec"), namespace)
    return namespace["run"]


class CountedBytes(bytes):
    copied = 0

    def __add__(self, other):
        type(self).copied += len(self) + len(other)
        return type(self)(super().__add__(other))


class Message(dict):
    pass


class Reader(asyncio.StreamReader):
    def __init__(self, counted=False):
        super().__init__()
        self.idle = asyncio.Event()
        self.sizes = []
        self.counted = counted
        self.after_read = lambda: None

    async def readexactly(self, size):
        self.sizes.append(size)
        if size == 4 and not self._buffer:
            self.idle.set()
        data = await super().readexactly(size)
        self.after_read()
        return CountedBytes(data) if self.counted else data


class Harness:
    def __init__(self, counted=False):
        self._stopped = False
        self._stopped_future = asyncio.get_running_loop().create_future()
        self.on_error_future = asyncio.get_running_loop().create_future()
        self.reader = Reader(counted)
        self.messages = []
        self.on_message = self.messages.append
        self.communications = 0
        self._proc = SimpleNamespace(
            stdout=self.reader, stdin=object(), communicate=self.communicate)

    def deserialize_message(self, wire):
        return Message(json.loads(wire))

    async def communicate(self):
        self.communications += 1

    def feed(self, value):
        wire = json.dumps(value, ensure_ascii=False).encode()
        self.reader.feed_data(len(wire).to_bytes(4, "little") + wire)
        return len(wire)

    async def stop(self, task):
        self._stopped = True
        self.reader.feed_eof()
        try:
            await asyncio.wait_for(task, 2)
        finally:
            if self.on_error_future.done():
                self.on_error_future.exception()


async def idle_sample(run, size, counted=False):
    harness = Harness(counted)
    references = []
    harness.on_message = lambda message: references.append(weakref.ref(message))
    wire_bytes = harness.feed({"body": "x" * size})
    CountedBytes.copied = 0
    task = asyncio.create_task(run(harness))
    try:
        await asyncio.wait_for(harness.reader.idle.wait(), 3)
        gc.collect()
        locals_ = task.get_coro().cr_frame.f_locals
        return {
            "wire_bytes": wire_bytes,
            "decoded_message_retained": references[0]() is not None,
            "transport_local_bytes": sum(len(v) for v in locals_.values()
                                         if isinstance(v, (bytes, bytearray))),
            "concatenation_bytes": CountedBytes.copied,
        }
    finally:
        await harness.stop(task)


class TransportTests(unittest.IsolatedAsyncioTestCase):
    async def test_exact_unicode_and_multiple_frames(self):
        values = [{}, {"body": "π 😀 \\ \"\n" * 20000}, {"done": True}]
        for run in (original_run(), run_linear):
            harness = Harness()
            for value in values:
                harness.feed(value)
            task = asyncio.create_task(run(harness))
            try:
                await asyncio.wait_for(harness.reader.idle.wait(), 3)
                self.assertEqual(harness.messages, values)
                self.assertLessEqual(max(harness.reader.sizes), 32768)
            finally:
                await harness.stop(task)

    async def test_partial_eof_preserves_error_and_shutdown(self):
        for run in (original_run(), run_linear):
            for wire in (b"", b"\x10\x00", b"\x10\x00\x00\x00abc"):
                harness = Harness()
                harness.reader.feed_data(wire)
                harness.reader.feed_eof()
                await run(harness)
                self.assertEqual(str(harness.on_error_future.exception()),
                                 "Connection closed while reading from the driver")
                self.assertEqual(harness.messages, [])
                self.assertEqual(harness.communications, 1)
                self.assertTrue(harness._stopped_future.done())

    async def test_stop_during_header_or_body_does_not_dispatch(self):
        for run in (original_run(), run_linear):
            for stop_read in (1, 2):
                harness = Harness()
                harness.feed({"body": "x" * 100000})
                def stop():
                    if len(harness.reader.sizes) == stop_read:
                        harness._stopped = True
                harness.reader.after_read = stop
                await run(harness)
                self.assertEqual(harness.messages, [])
                self.assertFalse(harness.on_error_future.done())
                self.assertTrue(harness._stopped_future.done())

    async def test_cancellation_releases_shutdown_waiters(self):
        for run in (original_run(), run_linear):
            for wire in (b"", b"\x10\x00\x00\x00"):
                harness = Harness()
                harness.reader.feed_data(wire)
                task = asyncio.create_task(run(harness))
                await asyncio.sleep(0)
                task.cancel()
                with self.assertRaises(asyncio.CancelledError):
                    await task
                self.assertTrue(harness._stopped_future.done())
                self.assertFalse(harness.on_error_future.done())

    async def test_decoder_and_dispatch_errors_release_shutdown_waiters(self):
        for run in (original_run(), run_linear):
            harness = Harness()
            harness.reader.feed_data(b"\x01\x00\x00\x00!")
            with self.assertRaises(json.JSONDecodeError):
                await run(harness)
            self.assertTrue(harness._stopped_future.done())
            harness = Harness()
            harness.feed({"ok": True})
            def fail(_):
                raise RuntimeError("fixture dispatch")
            harness.on_message = fail
            with self.assertRaisesRegex(RuntimeError, "fixture dispatch"):
                await run(harness)
            self.assertTrue(harness._stopped_future.done())

    async def test_copy_volume_and_delivered_message_release(self):
        before = await idle_sample(original_run(), 2 * 1024**2, counted=True)
        after = await idle_sample(run_linear, 2 * 1024**2, counted=True)
        self.assertEqual(before["wire_bytes"], after["wire_bytes"])
        self.assertGreater(before["concatenation_bytes"], 30 * before["wire_bytes"])
        self.assertEqual(after["concatenation_bytes"], 0)
        self.assertTrue(before["decoded_message_retained"])
        self.assertFalse(after["decoded_message_retained"])
        self.assertGreaterEqual(before["transport_local_bytes"], before["wire_bytes"])
        self.assertEqual(after["transport_local_bytes"], 0)


class InstallTests(unittest.TestCase):
    def test_embedded_transport_matches_the_tested_helper(self):
        names = {"run_linear", "install", "SOURCE_SHA256", "RUN_SHA256"}
        def selected(path):
            result = {}
            for node in ast.parse(path.read_text()).body:
                if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                    name = node.name
                elif isinstance(node, ast.Assign) and isinstance(node.targets[0], ast.Name):
                    name = node.targets[0].id
                else:
                    continue
                if name in names:
                    result[name] = ast.dump(node, include_attributes=False)
            return result
        root = Path(__file__).parents[1]
        helper = selected(root / "linear_playwright_transport.py")
        self.assertEqual(set(helper), names)
        self.assertEqual(selected(root / "camoufox_host.py"), helper)

    def test_installation_is_pinned_and_idempotent(self):
        from playwright._impl._transport import PipeTransport
        original = PipeTransport.run
        with patch.object(PipeTransport, "run", original):
            self.assertEqual(install(), "applied")
            self.assertIs(PipeTransport.run, run_linear)
            self.assertEqual(install(), "already_applied")
        self.assertIs(PipeTransport.run, original)

    def test_mismatched_source_is_not_changed(self):
        from playwright._impl._transport import PipeTransport
        original = PipeTransport.run
        with patch("linear_playwright_transport.SOURCE_SHA256", "different"):
            self.assertEqual(install(), "unsupported")
        self.assertIs(PipeTransport.run, original)

    def test_existing_replacement_is_not_overwritten(self):
        from playwright._impl._transport import PipeTransport
        async def replacement(self):
            pass
        with patch.object(PipeTransport, "run", replacement):
            self.assertEqual(install(), "unsupported")
            self.assertIs(PipeTransport.run, replacement)

    def test_missing_transport_class_is_unsupported(self):
        from playwright._impl import _transport
        with patch.object(_transport, "PipeTransport", None):
            self.assertEqual(install(), "unsupported")


if __name__ == "__main__":
    unittest.main()
