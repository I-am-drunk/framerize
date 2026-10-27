"""Private protocol ownership with actual Host classes and inert browser ports."""
import asyncio
import json
import unittest

import control_protocol_fixture as fixtures
from control_protocol import ControlProtocol, Frames, MAX_FRAME
from test_control_mint import EXPECTED


class Writer:
    def __init__(self):
        self.replies = asyncio.Queue()
        self.closed = False

    def write(self, data):
        self.replies.put_nowait(json.loads(data))

    async def drain(self):
        pass

    def close(self):
        self.closed = True

    async def wait_closed(self):
        pass


class ProtocolTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.fixture = fixtures.OwnerFixture("test_success_joins_one_consumer_and_rejects_control_reuse")
        await self.fixture.asyncSetUp()
        self.egress = True
        self.reader, self.writer = asyncio.StreamReader(), Writer()
        f = self.fixture
        self.peer = ControlProtocol(f.host, f.identity, EXPECTED, None, lambda: self.egress)
        self.grant = dict(account="fixture", native_generation=1, host_generation="a" * 32,
            lease="00000000-0000-4000-8000-000000000001", device=EXPECTED.device, authority="ledger")
        self.task = None

    async def asyncTearDown(self):
        self.reader.feed_eof()
        self.fixture.release.set()
        self.fixture.browser.release.set()
        if self.task:
            await asyncio.gather(self.task, return_exceptions=True)
        await self.fixture.asyncTearDown()
        self.assertTrue(self.fixture.doCleanups(), "inert owner cleanup failed")

    def send(self, **value):
        self.reader.feed_data(json.dumps(dict(version=1, **value)).encode() + b"\n")

    async def reply(self):
        return await asyncio.wait_for(self.writer.replies.get(), 2)

    async def start(self):
        self.task = asyncio.create_task(self.peer.run(self.reader, self.writer))
        self.send(op="prepare", binding=self.peer.binding)
        self.assertEqual((await self.reply())["op"], "ready")

    def mint(self, budget=1000):
        self.send(op="mint", grant=self.grant, budget_ms=budget, request_ms=3000)

    def finish(self, joined=True):
        self.send(op="finish", grant=self.grant, joined=joined, risk_refused=False)

    async def test_token_delivery_holds_consumer_until_finish_and_physical_close(self):
        await self.start()
        self.mint()
        self.assertEqual((await self.reply())["token"], "fixture-token")
        self.assertTrue(self.fixture.host._control_busy)
        self.assertFalse(self.peer.control.done())
        self.assertTrue(self.fixture.browser.connected)
        self.fixture.browser.release.clear()
        self.finish()
        await asyncio.wait_for(self.fixture.browser.called.wait(), 2)
        self.assertTrue(self.writer.replies.empty())
        self.assertFalse(self.task.done())
        self.fixture.browser.release.set()
        self.assertTrue((await self.reply())["cleanup_confirmed"])
        self.assertTrue(await self.task)
        self.assertFalse(self.fixture.browser.connected)
        self.assertTrue(self.writer.closed)

    async def test_cancel_waits_for_raw_rpc_without_publishing_token(self):
        self.fixture.page.evaluate = self.fixture.hold_evaluate
        await self.start()
        self.mint()
        await asyncio.wait_for(self.fixture.entered.wait(), 2)
        self.send(op="cancel", grant=self.grant)
        await asyncio.wait_for(self.fixture.host._control_cancelled.wait(), 2)
        self.assertFalse(self.task.done())
        self.assertTrue(self.writer.replies.empty())
        self.fixture.release.set()
        result = await self.reply()
        self.assertEqual(result["op"], "closed")
        self.assertTrue(result["cleanup_confirmed"])
        self.assertTrue(await self.task)

    async def test_mint_budget_retains_rpc_until_join(self):
        self.fixture.page.evaluate = self.fixture.hold_evaluate
        await self.start()
        self.mint(budget=20)
        await asyncio.wait_for(self.fixture.entered.wait(), 2)
        await asyncio.wait_for(self.fixture.host._control_cancelled.wait(), 2)
        self.assertFalse(self.task.done())
        self.fixture.release.set()
        reply = await self.reply()
        self.assertEqual(reply["reason"], "deadline")
        self.assertTrue(reply["cleanup_confirmed"])
        self.assertTrue(await self.task)

    async def test_unknown_native_completion_closes_but_never_confirms(self):
        await self.start()
        self.mint()
        await self.reply()
        self.finish(joined=False)
        self.assertFalse((await self.reply())["cleanup_confirmed"])
        self.assertFalse(await self.task)
        self.assertFalse(self.fixture.browser.connected)
        self.assertTrue(self.fixture.host._control_fenced)

    async def test_delivered_token_outlives_mint_budget_until_request_completion(self):
        await self.start()
        self.send(op="mint", grant=self.grant, budget_ms=100, request_ms=2000)
        self.assertEqual((await self.reply())["token"], "fixture-token")
        await asyncio.sleep(0.12)
        self.assertFalse(self.task.done())
        self.assertTrue(self.fixture.host._control_busy)
        self.assertTrue(self.writer.replies.empty())
        self.finish()
        self.assertTrue((await self.reply())["cleanup_confirmed"])
        self.assertTrue(await self.task)

    async def test_request_deadline_after_token_never_confirms_native_completion(self):
        await self.start()
        self.send(op="mint", grant=self.grant, budget_ms=100, request_ms=150)
        self.assertEqual((await self.reply())["token"], "fixture-token")
        self.assertFalse((await self.reply())["cleanup_confirmed"])
        self.assertFalse(await self.task)
        self.assertFalse(self.fixture.browser.connected)
        self.assertTrue(self.fixture.host._control_fenced)

    async def test_eof_after_token_closes_as_unconfirmed(self):
        await self.start()
        self.mint()
        await self.reply()
        self.reader.feed_eof()
        self.assertFalse((await self.reply())["cleanup_confirmed"])
        self.assertFalse(await self.task)
        self.assertFalse(self.fixture.browser.connected)

    async def test_changed_completion_lease_cannot_confirm(self):
        await self.start()
        self.mint()
        await self.reply()
        changed = dict(self.grant, lease="00000000-0000-4000-8000-000000000002")
        self.send(op="finish", grant=changed, joined=True, risk_refused=False)
        self.assertFalse((await self.reply())["cleanup_confirmed"])
        self.assertFalse(await self.task)

    async def test_false_egress_witness_refuses_before_observation(self):
        self.egress = False
        self.task = asyncio.create_task(self.peer.run(self.reader, self.writer))
        self.send(op="prepare", binding=self.peer.binding)
        self.assertFalse((await self.reply())["cleanup_confirmed"])
        self.assertFalse(await self.task)
        self.assertEqual(self.fixture.page.calls, [])

    async def test_changed_egress_after_mint_cannot_confirm_finish(self):
        await self.start()
        self.mint()
        await self.reply()
        self.egress = False
        self.finish()
        self.assertFalse((await self.reply())["cleanup_confirmed"])
        self.assertFalse(await self.task)

    async def test_repeated_cancel_of_server_waiter_still_joins_actual_work(self):
        self.fixture.page.evaluate = self.fixture.hold_evaluate
        await self.start()
        self.mint()
        await asyncio.wait_for(self.fixture.entered.wait(), 2)
        self.task.cancel()
        await asyncio.sleep(0)
        self.task.cancel()
        await asyncio.sleep(0)
        self.assertFalse(self.task.done())
        self.fixture.release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(asyncio.shield(self.task), 2)
        self.assertFalse(self.fixture.browser.connected)
        self.assertTrue(self.fixture.host._control_fenced)
        self.assertFalse((await self.reply())["cleanup_confirmed"])

    async def test_changed_session_refuses_before_observer(self):
        self.task = asyncio.create_task(self.peer.run(self.reader, self.writer))
        self.send(op="prepare", binding=dict(self.peer.binding, session="wrong-fixture-session"))
        self.assertFalse((await self.reply())["cleanup_confirmed"])
        self.assertFalse(await self.task)
        self.assertEqual(self.fixture.page.calls, [])

    async def test_boolean_generation_is_not_an_integer_grant(self):
        await self.start()
        self.send(op="mint", grant=dict(self.grant, native_generation=True),
                  budget_ms=100, request_ms=1000)
        self.assertFalse((await self.reply())["cleanup_confirmed"])
        self.assertFalse(await self.task)
        self.assertEqual(self.fixture.page.calls, [])


class FrameTests(unittest.IsolatedAsyncioTestCase):
    async def test_fragmented_frame_over_four_kib_keeps_all_bytes(self):
        reader = asyncio.StreamReader()
        frames = Frames(reader, Writer())
        value = dict(version=1, token="x" * 16384)
        raw = json.dumps(value).encode() + b"\n"
        reader.feed_data(raw[:4095])
        pending = asyncio.create_task(frames.read())
        await asyncio.sleep(0)
        self.assertFalse(pending.done())
        reader.feed_data(raw[4095:])
        self.assertEqual(await asyncio.wait_for(pending, 2), value)

    async def test_duplicate_keys_bad_versions_nonfinite_and_overflow_refuse(self):
        for data in (b'{"version":1,"version":1}\n', b'{"version":true}\n',
                     b'{"version":1,"x":NaN}\n', b'x' * (MAX_FRAME + 1)):
            with self.subTest(size=len(data)):
                reader = asyncio.StreamReader()
                reader.feed_data(data)
                reader.feed_eof()
                with self.assertRaises(Exception):
                    await Frames(reader, Writer()).read()


if __name__ == "__main__":
    unittest.main()
