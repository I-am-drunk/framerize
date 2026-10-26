"""Borrowed page ownership while real host RPCs use inert browser ports."""
import asyncio
import unittest
from unittest.mock import AsyncMock, patch

import test_page_acquisition as acquisition


class Evaluation:
    def __init__(self):
        self.call = acquisition.Call()
        self.expressions = []
        self.value = object()

    async def __call__(self, expression):
        self.expressions.append(expression)
        await self.call()
        return self.value


class PageUseTests(unittest.IsolatedAsyncioTestCase):
    finish = acquisition.PageAcquisitionTests.finish
    block = acquisition.PageAcquisitionTests.block
    start = acquisition.PageAcquisitionTests.start
    assert_waiting = acquisition.PageAcquisitionTests.assert_waiting
    assert_idle = acquisition.PageAcquisitionTests.assert_idle

    def setUp(self):
        acquisition.PageAcquisitionTests.setUp(self)
        cap = patch.object(self.module, "POOL_CAP", 1)
        cap.start()
        self.addCleanup(cap.stop)

    def account(self, name="a"):
        acc = acquisition.PageAcquisitionTests.account(self, name)
        for page in (acc.ctx.ambient, acc.ctx.next_page):
            page.evaluate = Evaluation()
        acc.ctx.cookies = AsyncMock(side_effect=AssertionError("unexpected cookie read"))
        return acc

    def command(self, name="a", cmd="eval", **fields):
        request = dict(cmd=cmd, account=name, expr="fixture expression", **fields)
        task = self.start(self.host.handle(request))
        key = len(self.waiters)
        self.host._command_tasks[key] = task
        task.add_done_callback(lambda _: self.host._command_tasks.pop(key, None))
        return task

    def assert_available(self, acc):
        self.assertFalse(acc.borrowed)
        self.assertEqual(acc.pool.qsize(), acc.size)
        self.assert_idle(acc)

    async def test_cancelled_eval_retains_page_until_raw_call_settles(self):
        acc = self.account()
        page = acc.ctx.ambient
        gate = self.block(page.evaluate.call)
        request = self.command()
        await asyncio.wait_for(gate.entered.wait(), 1)
        request.cancel()
        await self.assert_waiting(request)
        self.assertEqual(acc.borrowed, {page})
        self.assertTrue(acc.pool.empty())
        self.assertFalse(await self.host._close_account("a"))
        gate.release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(request, 1)
        self.assert_available(acc)
        self.assertIs(await asyncio.wait_for(self.command(), 1), page.evaluate.value)
        self.assertEqual(page.evaluate.call.calls, 2)
        self.assertEqual(acc.ctx.creation.calls, 0)
        self.assert_available(acc)

    async def test_cancelled_login_stops_followups_after_success_or_failure(self):
        for failure in (False, True):
            with self.subTest(late_failure=failure):
                name = str(failure)
                acc = self.account(name)
                evaluation = acc.ctx.ambient.evaluate
                evaluation.value = {"status": 200, "body": "{}"}
                gate = self.block(evaluation.call)
                if failure:
                    gate.failure = RuntimeError("fixture late failure")
                request = self.command(name, cmd="complete", token="fixture-token")
                await asyncio.wait_for(gate.entered.wait(), 1)
                request.cancel()
                await self.assert_waiting(request)
                gate.release.set()
                with self.assertRaises(asyncio.CancelledError):
                    await asyncio.wait_for(request, 1)
                self.assertEqual(len(evaluation.expressions), 1)
                acc.ctx.cookies.assert_not_awaited()
                self.assert_available(acc)

    async def test_repeated_cancellation_blocks_reborrow_not_other_account(self):
        acc, other = self.account(), self.account("b")
        gate = self.block(acc.ctx.ambient.evaluate.call)
        first = self.command()
        await asyncio.wait_for(gate.entered.wait(), 1)
        second = self.command()
        for _ in range(3):
            first.cancel()
            await self.assert_waiting(first)
        await self.assert_waiting(second)
        self.assertEqual(gate.calls, 1)
        self.assertIs(await asyncio.wait_for(self.command("b"), 1),
                      other.ctx.ambient.evaluate.value)
        self.assert_available(other)
        gate.release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(first, 1)
        self.assertIs(await asyncio.wait_for(second, 1), acc.ctx.ambient.evaluate.value)
        self.assertEqual(gate.calls, 2)
        self.assertEqual(acc.ctx.creation.calls, 0)
        self.assert_available(acc)

    async def test_release_does_not_join_unrelated_same_account_rpc(self):
        acc = self.account()
        foreign_gate = self.block(acquisition.Call())
        foreign = self.start(self.host._rpc("a", foreign_gate))
        await asyncio.wait_for(foreign_gate.entered.wait(), 1)
        gate = self.block(acc.ctx.ambient.evaluate.call)
        request = self.command()
        await asyncio.wait_for(gate.entered.wait(), 1)
        request.cancel()
        await self.assert_waiting(request)
        gate.release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(request, 1)
        self.assertFalse(foreign.done())
        self.assertFalse(acc.borrowed)
        self.assertIs(await asyncio.wait_for(self.command(), 1), acc.ctx.ambient.evaluate.value)
        foreign_gate.release.set()
        await asyncio.wait_for(foreign, 1)
        self.assert_available(acc)

    async def test_normal_result_and_errors_keep_page_reusable(self):
        acc = self.account()
        evaluation = acc.ctx.ambient.evaluate
        self.assertIs(await asyncio.wait_for(self.command(), 1), evaluation.value)
        self.assert_available(acc)
        error = RuntimeError("fixture evaluation failed")
        evaluation.call.failure = error
        with self.assertRaises(RuntimeError) as raised:
            await asyncio.wait_for(self.command(), 1)
        self.assertIs(raised.exception, error)
        self.assert_available(acc)
        evaluation.call.failure = None
        evaluation.value = {"fixture": "reused"}
        self.assertIs(await asyncio.wait_for(self.command(), 1), evaluation.value)
        self.assertEqual(evaluation.call.calls, 3)
        self.assertEqual(acc.ctx.creation.calls, 0)
        self.assert_available(acc)

    async def test_another_free_page_progresses_while_cancelled_use_settles(self):
        acc = self.account()
        gate = self.block(acc.ctx.ambient.evaluate.call)
        with patch.object(self.module, "POOL_CAP", 2):
            first = self.command()
            await asyncio.wait_for(gate.entered.wait(), 1)
            first.cancel()
            await self.assert_waiting(first)
            self.assertIs(await asyncio.wait_for(self.command(), 1),
                          acc.ctx.next_page.evaluate.value)
            self.assertFalse(first.done())
            self.assertEqual(acc.borrowed, {acc.ctx.ambient})
            self.assertEqual(acc.ctx.creation.calls, 1)
            self.assertEqual(acc.size, 2)
            gate.release.set()
            with self.assertRaises(asyncio.CancelledError):
                await asyncio.wait_for(first, 1)
        self.assert_available(acc)

    async def test_shutdown_joins_cancelled_page_rpc_before_browser_close(self):
        acc = self.account()
        gate = self.block(acc.ctx.ambient.evaluate.call)
        request = self.command()
        await asyncio.wait_for(gate.entered.wait(), 1)
        request.cancel()
        closing = self.start(self.host.close())
        await self.assert_waiting(closing)
        request.cancel()
        await self.assert_waiting(request)
        self.assertEqual(acc.borrowed, {acc.ctx.ambient})
        self.assertEqual(acc.browser.calls, 0)
        gate.release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(request, 1)
        await asyncio.wait_for(closing, 1)
        self.assertEqual(acc.browser.calls, 1)
        self.assertTrue(acc.browser.acknowledged.is_set())
        self.assertEqual(acc.lifecycle.phase, "absent")
        self.assertFalse(self.host.accounts)
        self.assertFalse(acc.borrowed)
        self.assert_idle(acc)
