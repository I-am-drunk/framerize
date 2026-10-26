import asyncio
import json
from pathlib import Path
import sys
import tempfile
import time
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_host import app, supplied


def request(suffix="a"):
    return dict(cmd="send_supplied", version=1,
        account="00000000-0000-0000-0000-00000000000" + ("1" if suffix == "a" else "2"),
        user="user-" + suffix, device="device-" + suffix, bearer="bearer-" + suffix,
        cookie_header=f"session=login-{suffix}; __cuid=device-{suffix}",
        path="/ai/v3/responses/", wire=json.dumps({"castleToken": "token-" + suffix}),
        expires_unix_ms=int(time.time() * 1000) + 10000,
        timeout_ms=3000, response_limit=1024, proxy=None)


class Context:
    def __init__(self):
        self.cookies, self.starts, self.listeners = [], [], {}
        self.closed, self.entered, self.reply, self.close_entered, self.close_gate = [asyncio.Event() for _ in range(5)]
        self.reply.set()
        self.close_gate.set()
        self._impl_obj = SimpleNamespace(_channel=SimpleNamespace(send=self.close))

    async def close(self, *args):
        self.close_entered.set()
        await self.close_gate.wait()
        self.closed.set()
        self.reply.set()
        for callback in tuple(self.listeners.get("close", [])):
            callback()

    def on(self, event, callback):
        self.listeners.setdefault(event, []).append(callback)

    def remove_listener(self, event, callback):
        self.listeners[event].remove(callback)

    async def route(self, *_):
        pass

    async def add_cookies(self, cookies):
        self.cookies = cookies

    async def add_init_script(self, script):
        self.init_script = script

    async def new_page(self):
        return self

    async def goto(self, url, **kwargs):
        self.url = url

    async def evaluate(self, script, args):
        if script == supplied.START:
            self.starts.append(args)
            self.entered.set()
            return args[1]
        await self.reply.wait()
        if self.closed.is_set():
            raise RuntimeError("closed synthetic context")
        if args[1] == 0:
            return {"kind": "missing", "content_type": None}
        return dict(kind="chunk", sequence=0, offset=0, next=2, body="ok", done=True,
            status=200, usage="fixture", peak_units=2, content_type="text/plain")


class SuppliedTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        override = patch.object(app, "ROOT", directory.name)
        override.start()
        self.addCleanup(override.stop)
        self.host = supplied.SuppliedHost()
        self.contexts, self.planned = [], []
        async def create(**kwargs):
            context = self.planned.pop(0) if self.planned else Context()
            context.options = kwargs
            self.contexts.append(context)
            return context
        self.host.browser = SimpleNamespace(new_context=AsyncMock(side_effect=create))
        self.host.receipt = SimpleNamespace(close=AsyncMock())
        self.host._start_shared = AsyncMock()

    async def asyncTearDown(self):
        for context in self.contexts:
            context.reply.set()
            context.close_gate.set()
        if self.host.jobs:
            await asyncio.gather(*tuple(self.host.jobs), return_exceptions=True)
        await self.host.close()

    async def wait(self, event):
        await asyncio.wait_for(event.wait(), 1)

    async def test_result_waits_for_context_close_and_preserves_supplied_token(self):
        context = Context()
        context.close_gate.clear()
        self.planned.append(context)
        data = request()
        task = asyncio.create_task(self.host.handle(data))
        await self.wait(context.close_entered)
        self.assertFalse(task.done())
        payload, _ = context.starts[0]
        self.assertEqual(payload["wire"], data["wire"])
        self.assertEqual(payload["device"], data["device"])
        self.assertNotIn("cookie_header", payload)
        self.assertNotIn("login-a", json.dumps(payload))
        self.assertEqual({v["name"]: v["value"] for v in context.cookies},
                         {"session": "login-a", "__cuid": "device-a"})
        context.close_gate.set()
        result = await task
        self.assertTrue(result["cleanup_confirmed"])
        self.assertEqual(result["response"]["body"], "ok")
        self.assertTrue(context.closed.is_set())
        self.assertEqual(self.host._start_shared.await_count, 1)

    async def test_cancel_keeps_the_original_request_owned_until_close_acknowledgement(self):
        context = Context()
        context.reply.clear()
        context.close_gate.clear()
        self.planned.append(context)
        task = asyncio.create_task(self.host.handle(request()))
        await self.wait(context.entered)
        task.cancel()
        await self.wait(context.close_entered)
        self.assertFalse(task.done())
        self.assertEqual(len(self.host.jobs), 1)
        context.close_gate.set()
        result = await asyncio.wait_for(task, 1)
        self.assertTrue(result["cleanup_confirmed"])
        self.assertIsNone(result["response"])
        self.assertTrue(context.closed.is_set())

    async def test_expired_and_mismatched_devices_never_create_a_context(self):
        expired, mismatch = request(), request()
        expired["expires_unix_ms"] = 0
        mismatch["device"] = "other-device"
        for data in (expired, mismatch):
            result = await self.host.handle(data)
            self.assertIsNone(result["response"])
        self.assertEqual(self.contexts, [])

    async def test_concurrent_accounts_keep_separate_cookies_and_closure(self):
        first, second = Context(), Context()
        first.reply.clear()
        second.reply.clear()
        self.planned.extend((first, second))
        a = asyncio.create_task(self.host.handle(request("a")))
        b = asyncio.create_task(self.host.handle(request("b")))
        await self.wait(first.entered)
        await self.wait(second.entered)
        self.assertEqual(first.starts[0][0]["device"], "device-a")
        self.assertEqual(second.starts[0][0]["device"], "device-b")
        self.assertNotEqual(first.cookies, second.cookies)
        a.cancel()
        self.assertTrue((await a)["cleanup_confirmed"])
        self.assertFalse(second.closed.is_set())
        self.assertFalse(b.done())
        second.reply.set()
        self.assertEqual((await b)["response"]["body"], "ok")
        self.assertEqual(self.host._start_shared.await_count, 1)

    async def test_uncertain_context_creation_cannot_report_cleanup(self):
        self.host.browser.new_context.side_effect = RuntimeError("unconfirmed creation")
        result = await self.host.handle(request())
        self.assertFalse(result["cleanup_confirmed"])
        self.assertIsNone(result["response"])

    async def test_close_failure_discards_success_and_refuses_further_work(self):
        context = Context()
        context._impl_obj._channel.send = AsyncMock(side_effect=RuntimeError("no close ack"))
        self.planned.append(context)
        result = await self.host.handle(request())
        self.assertFalse(result["cleanup_confirmed"])
        self.assertIsNone(result["response"])
        with self.assertRaises(RuntimeError):
            await self.host.handle(request("b"))
        self.assertEqual(len(self.contexts), 1)

    async def test_protocol_failures_never_serialize_arbitrary_exception_text(self):
        self.host.prepare = AsyncMock(side_effect=RuntimeError("private-cookie-value"))
        writer = SimpleNamespace(send=AsyncMock())
        await self.host._serve(dict(cmd="prepare_supplied", id=1), writer)
        value = writer.send.await_args.args[0]
        self.assertNotIn("private-cookie-value", json.dumps(value))
        self.assertFalse(value["result"]["cleanup_confirmed"])

    async def test_completed_exchange_timeout_preserves_confirmed_cleanup(self):
        from framerize_host.browser.responses import ResponseDeadline
        self.host._exchange = AsyncMock(side_effect=ResponseDeadline("exchange deadline"))
        result = await self.host.handle(request())
        self.assertEqual(result, dict(version=1, response=None, cleanup_confirmed=True))
        self.assertTrue(self.contexts[0].closed.is_set())
        self.assertFalse(self.host.failed)
        self.assertFalse(self.host.jobs or self.host.exchanges or self.host.uncertain)


if __name__ == "__main__":
    unittest.main()
