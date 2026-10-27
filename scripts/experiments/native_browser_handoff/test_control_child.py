"""Private child setup and protocol integration with inert dependency ports."""
import asyncio
import json
import unittest

import test_control_preparation as preparation
from test_control_protocol import Writer
from control_child import PreparedControlChild


class ChildTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        fixture_type = type("Fixture", (preparation.PreparationTests,),
            {"_callCleanup": unittest.TestCase._callCleanup})
        self.fixture = fixture_type("test_preparation_binds_one_page_without_mint_or_background_work")
        await self.fixture.asyncSetUp()
        self.reader, self.writer = asyncio.StreamReader(), Writer()
        self.egress, self.attest_calls = True, 0
        self.entered, self.release = asyncio.Event(), asyncio.Event()
        self.release.set()
        self.child = self.make_child()
        self.tasks = []

    def make_child(self, **changes):
        args = dict(attest_egress=self.attest, proxy_revision=7,
            prepare_seconds=5, host_factory=lambda: self.fixture.host)
        args.update(changes)
        return PreparedControlChild(**args)

    async def attest(self, host, identity, expected, proxy, stopped, deadline):
        self.attest_calls += 1
        self.assertIs(host, self.fixture.host)
        self.assertEqual(identity.account, preparation.ACCOUNT)
        self.assertEqual(expected, preparation.EXPECTED)
        self.assertIsNone(proxy)
        self.entered.set()
        await self.release.wait()
        return lambda: self.egress

    def frame(self, **value):
        return json.dumps(dict(version=1, **value)).encode() + b"\n"

    def binding(self):
        return dict(vars(preparation.EXPECTED), account=preparation.ACCOUNT, proxy=None)

    def start(self):
        task = asyncio.create_task(self.child.run(self.reader, self.writer))
        self.tasks.append(task)
        return task

    async def reply(self):
        return await asyncio.wait_for(self.writer.replies.get(), 2)

    async def ready(self):
        task = self.start()
        self.reader.feed_data(self.frame(op="prepare", binding=self.binding()))
        self.assertEqual((await self.reply())["op"], "ready")
        return task

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

    async def test_coalesced_prepare_and_shutdown_preserve_unread_bytes(self):
        task = self.start()
        self.reader.feed_data(self.frame(op="prepare", binding=self.binding())
            + self.frame(op="shutdown"))
        self.assertEqual((await self.reply())["op"], "ready")
        closed = await self.reply()
        self.assertEqual(closed["op"], "closed")
        self.assertTrue(closed["cleanup_confirmed"])
        self.assertTrue(await task)
        self.assertTrue(self.writer.closed and self.fixture.ports.stopped)
        self.assertEqual(self.attest_calls, 1)

    async def test_ready_waits_for_actual_attester_completion(self):
        self.release.clear()
        task = self.start()
        self.reader.feed_data(self.frame(op="prepare", binding=self.binding()))
        await asyncio.wait_for(self.entered.wait(), 1)
        self.assertTrue(self.writer.replies.empty())
        self.assertFalse(task.done())
        self.release.set()
        self.assertEqual((await self.reply())["op"], "ready")
        self.reader.feed_data(self.frame(op="shutdown"))
        self.assertTrue((await self.reply())["cleanup_confirmed"])
        self.assertTrue(await task)

    def grant(self):
        return dict(account=preparation.ACCOUNT, native_generation=1,
            host_generation="a" * 32, lease="00000000-0000-4000-8000-000000000001",
            device=preparation.EXPECTED.device, authority="ledger")

    async def test_mint_holds_browser_until_native_finish_and_physical_close(self):
        task = await self.ready()
        self.reader.feed_data(self.frame(op="mint", grant=self.grant(),
            budget_ms=1000, request_ms=3000))
        self.assertEqual((await self.reply())["token"], "fixture-token")
        host = self.fixture.host
        self.assertTrue(host._control_busy)
        browser = self.fixture.ports.browsers[0]
        browser.release.clear()
        self.reader.feed_data(self.frame(op="finish", grant=self.grant(),
            joined=True, risk_refused=False))
        await asyncio.wait_for(browser.called.wait(), 1)
        self.assertTrue(self.writer.replies.empty())
        browser.release.set()
        self.assertTrue((await self.reply())["cleanup_confirmed"])
        self.assertTrue(await task)

    async def test_false_egress_witness_closes_without_ready_or_mint(self):
        self.egress = False
        task = self.start()
        self.reader.feed_data(self.frame(op="prepare", binding=self.binding()))
        reply = await self.reply()
        self.assertEqual(reply["op"], "closed")
        self.assertTrue(reply["cleanup_confirmed"])
        self.assertFalse(await task)
        self.assertEqual(self.fixture.ports.browsers[0].contexts[0].pages[0].calls, [])
        self.assertTrue(self.fixture.ports.stopped)

    async def test_missing_attester_is_not_replaced_with_success(self):
        self.child = self.make_child(attest_egress=None)
        task = self.start()
        self.assertEqual((await self.reply())["op"], "closed")
        self.assertFalse(await task)
        self.assertEqual(self.fixture.ports.calls, [])
        self.assertIsNone(self.child.host)

    async def test_empty_input_deadline_launches_nothing(self):
        self.child = self.make_child(prepare_seconds=0.05)
        task = self.start()
        self.assertEqual((await self.reply())["op"], "closed")
        self.assertFalse(await task)
        self.assertEqual(self.fixture.ports.calls, [])

    async def test_malformed_binding_refuses_before_host_creation(self):
        for change in (dict(account="../escape"), dict(session=""),
                       dict(proxy=[]), dict(extra=True)):
            with self.subTest(fields=list(change)):
                self.reader, self.writer = asyncio.StreamReader(), Writer()
                self.child = self.make_child()
                task = self.start()
                binding = self.binding()
                binding.update(change)
                self.reader.feed_data(self.frame(op="prepare", binding=binding))
                self.assertEqual((await self.reply())["op"], "closed")
                self.assertFalse(await task)
                self.assertIsNone(self.child.host)
                self.assertEqual(self.fixture.ports.calls, [])

    async def test_cancelled_initial_read_closes_io_without_launch(self):
        task = self.start()
        await asyncio.sleep(0)
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertTrue(self.writer.closed)
        self.assertEqual(self.fixture.ports.calls, [])

    async def test_cancel_during_preparation_joins_late_browser_operations(self):
        ports = self.fixture.ports
        ports.hold("context")
        task = self.start()
        self.reader.feed_data(self.frame(op="prepare", binding=self.binding()))
        await asyncio.wait_for(ports.entered["context"].wait(), 1)
        task.cancel()
        await asyncio.wait_for(self.child.stopped.wait(), 1)
        self.assertFalse(task.done())
        self.assertEqual(ports.browsers[0].calls, 0)
        ports.release["context"].set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(asyncio.shield(task), 2)
        self.assertTrue(ports.stopped and self.writer.closed)
        self.assertEqual(self.attest_calls, 0)

    async def test_cancel_during_attestation_joins_before_browser_close(self):
        self.release.clear()
        task = self.start()
        self.reader.feed_data(self.frame(op="prepare", binding=self.binding()))
        await asyncio.wait_for(self.entered.wait(), 1)
        task.cancel()
        await asyncio.wait_for(self.child.stopped.wait(), 1)
        self.assertFalse(task.done())
        self.assertEqual(self.fixture.ports.browsers[0].calls, 0)
        self.release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(asyncio.shield(task), 2)
        self.assertTrue(self.fixture.ports.stopped)

    async def test_attestation_deadline_waits_for_actual_settlement(self):
        self.release.clear()
        self.child = self.make_child(prepare_seconds=0.15)
        task = self.start()
        self.reader.feed_data(self.frame(op="prepare", binding=self.binding()))
        await asyncio.wait_for(self.entered.wait(), 1)
        await asyncio.wait_for(self.child.stopped.wait(), 1)
        self.assertFalse(task.done())
        self.assertEqual(self.fixture.ports.browsers[0].calls, 0)
        self.release.set()
        self.assertEqual((await self.reply())["op"], "closed")
        self.assertFalse(await task)
        self.assertTrue(self.fixture.ports.stopped)

    async def test_second_cancel_retains_cleanup_until_acknowledged(self):
        self.release.clear()
        task = self.start()
        self.reader.feed_data(self.frame(op="prepare", binding=self.binding()))
        await asyncio.wait_for(self.entered.wait(), 1)
        browser = self.fixture.ports.browsers[0]
        browser.release.clear()
        task.cancel()
        await asyncio.wait_for(self.child.stopped.wait(), 1)
        self.release.set()
        await asyncio.wait_for(browser.called.wait(), 1)
        task.cancel()
        await asyncio.sleep(0)
        self.assertFalse(task.done())
        browser.release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(asyncio.shield(task), 2)
        self.assertTrue(self.fixture.ports.stopped)

    async def test_egress_change_after_ready_refuses_mint(self):
        task = await self.ready()
        self.egress = False
        self.reader.feed_data(self.frame(op="mint", grant=self.grant(),
            budget_ms=1000, request_ms=3000))
        closed = await self.reply()
        self.assertEqual(closed["op"], "closed")
        self.assertFalse(closed["cleanup_confirmed"])
        self.assertFalse(await task)
        self.assertEqual(self.fixture.ports.browsers[0].contexts[0].pages[0].calls, [])

    async def test_setup_refusal_does_not_hide_failed_physical_close(self):
        self.release.clear()
        task = self.start()
        self.reader.feed_data(self.frame(op="prepare", binding=self.binding()))
        await asyncio.wait_for(self.entered.wait(), 1)
        self.fixture.allow_uncertain = True
        self.fixture.ports.browsers[0].failure = True
        self.egress = False
        self.release.set()
        closed = await self.reply()
        self.assertEqual(closed["reason"], "unconfirmed")
        self.assertFalse(closed["cleanup_confirmed"])
        self.assertFalse(await task)

    async def test_reuse_does_not_touch_the_active_protocol(self):
        task = await self.ready()
        spare = Writer()
        with self.assertRaises(preparation.ObservationRefused):
            await self.child.run(asyncio.StreamReader(), spare)
        self.assertFalse(spare.closed or self.writer.closed)
        self.reader.feed_data(self.frame(op="shutdown"))
        self.assertTrue((await self.reply())["cleanup_confirmed"])
        self.assertTrue(await task)

    async def test_attester_exception_is_payload_free_and_closes(self):
        async def fail(*args):
            raise RuntimeError("fixture-private-detail")
        self.child = self.make_child(attest_egress=fail)
        task = self.start()
        self.reader.feed_data(self.frame(op="prepare", binding=self.binding()))
        closed = await self.reply()
        self.assertNotIn("fixture-private-detail", json.dumps(closed))
        self.assertTrue(closed["cleanup_confirmed"])
        self.assertFalse(await task)
        self.assertTrue(self.fixture.ports.stopped)


if __name__ == "__main__":
    unittest.main()
