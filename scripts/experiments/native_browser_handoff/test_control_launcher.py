"""Launcher and pipe-exchange controls with inert browser dependencies."""
import asyncio
import json
from pathlib import Path
import subprocess
import sys
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, patch

import test_control_preparation as preparation
from test_control_protocol import Writer
from control_protocol import Frames
from control_mint import ObservationRefused
from control_launcher import PipeProbe, private_directory, run_child, run_stdio, validate_settings


class WireWriter(Writer):
    def is_closing(self):
        return self.closed


def frame(**value):
    return json.dumps(dict(version=1, **value)).encode() + b"\n"


class PipeTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.reader, self.writer = asyncio.StreamReader(), WireWriter()
        self.probe = PipeProbe(Frames(self.reader, self.writer))
        self.stopped, self.tasks = asyncio.Event(), []
        self.url = "https://egress.example/ip?framerize_probe=" + "a" * 32

    def begin(self):
        task = asyncio.create_task(self.probe.observe(self.url, 1000, self.stopped))
        self.tasks.append(task)
        return task

    async def asyncTearDown(self):
        self.reader.feed_eof()
        await asyncio.gather(*self.tasks, return_exceptions=True)

    async def request(self):
        return await asyncio.wait_for(self.writer.replies.get(), 1)

    def reply(self, **changes):
        value = dict(op="egress_result", url=self.url, ip="203.0.113.8")
        value.update(changes)
        self.reader.feed_data(frame(**value))

    async def test_exact_exchange_returns_only_the_validated_point_sample(self):
        task = self.begin()
        request = await self.request()
        self.assertEqual(request, dict(version=1, op="egress_probe", url=self.url, budget_ms=1000))
        self.reply(ip="2001:0db8::1")
        self.assertEqual(await task, dict(version=1, url=self.url, ip="2001:db8::1"))
        self.assertTrue(self.probe.pending.done() and self.probe.current())

    async def test_repeated_cancellation_waits_for_the_native_reply(self):
        task = self.begin()
        await self.request()
        for _ in range(2):
            task.cancel()
            await asyncio.sleep(0)
            self.assertFalse(task.done())
        self.reply()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertTrue(self.probe.pending.done() and self.stopped.is_set())
        self.assertFalse(self.probe.current())

    async def test_malformed_replies_refuse_and_invalidate_the_channel(self):
        for change in [dict(op="ready"), dict(url=self.url + "x"),
                       dict(ip="invalid"), dict(ip="fe80::1%eth0"), dict(extra=1)]:
            self.reader, self.writer = asyncio.StreamReader(), WireWriter()
            self.probe = PipeProbe(Frames(self.reader, self.writer))
            task = self.begin()
            await self.request()
            self.reply(**change)
            with self.assertRaises(ObservationRefused):
                await task
            self.assertFalse(self.probe.current())
            self.assertTrue(self.probe.pending.done())

    async def test_duplicate_keys_cannot_be_success(self):
        task = self.begin()
        await self.request()
        self.reader.feed_data(b'{"version":1,"version":1,"op":"egress_result"}\n')
        with self.assertRaises(ObservationRefused):
            await task
        self.assertFalse(self.probe.current())
        self.reader.feed_eof()
        with self.assertRaises(ObservationRefused):
            await self.probe.observe(self.url, 1000, self.stopped)

    async def test_lost_reply_is_joined_and_refused(self):
        task = self.begin()
        await self.request()
        self.reader.feed_eof()
        with self.assertRaises(ObservationRefused):
            await task
        self.assertTrue(self.probe.pending.done())
        self.assertFalse(self.probe.current())

    async def test_a_second_probe_is_refused_without_another_frame(self):
        task = self.begin()
        await self.request()
        self.reply()
        await task
        with self.assertRaises(ObservationRefused):
            await self.probe.observe(self.url, 1000, self.stopped)
        self.assertTrue(self.writer.replies.empty())


class SettingsTests(unittest.TestCase):
    def test_settings_require_a_matching_endpoint_and_finite_limits(self):
        endpoint = "https://egress.example/ip?flag&blank=&raw=%2f+"
        validate_settings(endpoint, 0, 180, 30, 180)
        for index, value in [(1, -1), (1, True), (2, 0), (2, float("nan")),
                             (3, 31), (4, float("inf")), (4, 181)]:
            args = [endpoint, 0, 180, 30, 180]
            args[index] = value
            with self.assertRaises(ValueError):
                validate_settings(*args)
        for endpoint in ["http://egress.example/ip", "https://EGRESS.example/ip",
                         "https://egress.example/ip#", "https://egress.example/ip?framerize_probe="]:
            with self.assertRaises((ValueError, ObservationRefused)):
                validate_settings(endpoint, 0, 180, 30, 180)

    def test_private_state_directory_refuses_existing_content_or_shared_mode(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory).resolve()
            self.assertEqual(private_directory(str(path)), path)
            path.chmod(0o755)
            with self.assertRaises(ValueError):
                private_directory(str(path))
            path.chmod(0o700)
            sentinel = path / "existing"
            sentinel.write_bytes(b"preserve")
            with self.assertRaises(ValueError):
                private_directory(str(path))
            self.assertEqual(sentinel.read_bytes(), b"preserve")


class LauncherTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        kind = type("Fixture", (preparation.PreparationTests,),
                    {"_callCleanup": unittest.TestCase._callCleanup})
        self.fixture = kind("test_preparation_binds_one_page_without_mint_or_background_work")
        await self.fixture.asyncSetUp()
        self.reader, self.writer = asyncio.StreamReader(), WireWriter()
        self.tasks, self.samples = [], []
        self.release = asyncio.Event()
        self.release.set()
        self.address = "203.0.113.8"
        override = patch.object(preparation.PreparedPage, "evaluate", self.evaluate)
        override.start()
        self.addCleanup(override.stop)
        self.acknowledged = asyncio.Event()
        original = PipeProbe._exchange
        async def exchange(native, url, milliseconds):
            result = await original(native, url, milliseconds)
            self.acknowledged.set()
            return result
        probe_override = patch.object(PipeProbe, "_exchange", exchange)
        probe_override.start()
        self.addCleanup(probe_override.stop)

    async def evaluate(self, script, request):
        self.samples.append(request["url"])
        await self.release.wait()
        return dict(version=1, url=request["url"], ip=self.address)

    def start(self):
        task = asyncio.create_task(run_child(Frames(self.reader, self.writer),
            "https://egress.example/ip?flag", 7, 5, 2, 5, host_factory=lambda: self.fixture.host))
        self.tasks.append(task)
        binding = dict(vars(preparation.EXPECTED), account=preparation.ACCOUNT, proxy=None)
        self.reader.feed_data(frame(op="prepare", binding=binding))
        return task

    async def reply(self):
        return await asyncio.wait_for(self.writer.replies.get(), 2)

    async def asyncTearDown(self):
        self.reader.feed_eof()
        self.release.set()
        for event in self.fixture.ports.release.values():
            event.set()
        for browser in self.fixture.ports.browsers:
            browser.release.set()
        await asyncio.gather(*self.tasks, return_exceptions=True)
        await self.fixture.asyncTearDown()
        self.assertTrue(self.fixture.doCleanups())

    async def test_coalesced_egress_reply_and_shutdown_preserve_buffered_input(self):
        task = self.start()
        probe = await self.reply()
        self.assertEqual(probe["op"], "egress_probe")
        self.reader.feed_data(frame(op="egress_result", url=probe["url"], ip=self.address)
                              + frame(op="shutdown"))
        self.assertEqual((await self.reply())["op"], "ready")
        closed = await self.reply()
        self.assertEqual(closed["op"], "closed")
        self.assertTrue(closed["cleanup_confirmed"])
        self.assertTrue(await task)
        self.assertEqual(self.samples, [probe["url"]])
        self.assertTrue(self.writer.closed and self.fixture.ports.stopped)

    async def test_different_egress_samples_refuse_before_ready(self):
        task = self.start()
        probe = await self.reply()
        self.reader.feed_data(frame(op="egress_result", url=probe["url"], ip="203.0.113.9"))
        closed = await self.reply()
        self.assertEqual(closed["op"], "closed")
        self.assertTrue(closed["cleanup_confirmed"])
        self.assertFalse(await task)
        self.assertTrue(self.writer.replies.empty() and self.fixture.ports.stopped)

    async def test_ready_waits_for_the_browser_sample_after_native_completion(self):
        self.release.clear()
        task = self.start()
        probe = await self.reply()
        self.reader.feed_data(frame(op="egress_result", url=probe["url"], ip=self.address))
        await asyncio.wait_for(self.acknowledged.wait(), 1)
        self.assertTrue(self.writer.replies.empty())
        self.assertFalse(task.done())
        self.release.set()
        self.assertEqual((await self.reply())["op"], "ready")
        self.reader.feed_data(frame(op="shutdown"))
        self.assertTrue((await self.reply())["cleanup_confirmed"])
        self.assertTrue(await task)

    async def test_cancelled_setup_joins_the_native_reply_before_closing(self):
        task = self.start()
        probe = await self.reply()
        for _ in range(2):
            task.cancel()
            await asyncio.sleep(0)
            self.assertFalse(task.done())
        self.reader.feed_data(frame(op="egress_result", url=probe["url"], ip=self.address))
        closed = await self.reply()
        self.assertEqual(closed["op"], "closed")
        self.assertTrue(closed["cleanup_confirmed"])
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertTrue(self.acknowledged.is_set() and self.fixture.ports.stopped)


class StdioTests(unittest.IsolatedAsyncioTestCase):
    async def test_repeated_cancellation_still_awaits_output_pipe_close(self):
        reader, writer = asyncio.StreamReader(), WireWriter()
        entered, release = asyncio.Event(), asyncio.Event()
        closed = []
        async def wait_closed():
            entered.set()
            await release.wait()
        writer.wait_closed = wait_closed
        transport = SimpleNamespace(close=lambda: closed.append(True))
        args = SimpleNamespace(egress_endpoint="https://example.com/ip", proxy_revision=0,
                               prepare_seconds=5, probe_seconds=2, witness_seconds=5)
        with patch("control_protocol.open_stdio", AsyncMock(return_value=(reader, writer, transport))), \
             patch("control_launcher.run_child", AsyncMock(return_value=True)):
            task = asyncio.create_task(run_stdio(args))
            try:
                await asyncio.wait_for(entered.wait(), 1)
                for _ in range(2):
                    task.cancel()
                    await asyncio.sleep(0)
                    self.assertFalse(task.done())
            finally:
                release.set()
                outcome = await asyncio.gather(task, return_exceptions=True)
        self.assertIsInstance(outcome[0], asyncio.CancelledError)
        self.assertEqual(closed, [True])
        self.assertTrue(writer.closed)


class EntryPointTests(unittest.TestCase):
    def test_real_entry_point_refuses_bad_binding_before_creating_browser_state(self):
        with tempfile.TemporaryDirectory() as directory:
            command = [sys.executable, "-B", str(Path(__file__).with_name("control_launcher.py")),
                "--egress-endpoint", "https://egress.example/ip",
                "--state-directory", str(Path(directory).resolve()),
                "--proxy-revision", "0", "--prepare-seconds", "2",
                "--probe-seconds", "1", "--witness-seconds", "2"]
            result = subprocess.run(command, input=frame(op="prepare", binding={}),
                stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                env={"PATH": "/usr/bin:/bin"}, timeout=5)
            self.assertEqual(result.returncode, 1)
            reply = json.loads(result.stdout)
            self.assertEqual(reply, dict(version=1, op="closed", grant=None,
                cleanup_confirmed=True, reason="rejected"))
            self.assertEqual(result.stderr, b"")
            self.assertEqual(list(Path(directory).iterdir()), [])
