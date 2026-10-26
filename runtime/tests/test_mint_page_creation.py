"""Dedicated-page creation and cancellation with inert browser ports."""
import asyncio
from dataclasses import replace
import unittest
from unittest.mock import AsyncMock, call, patch

import test_page_acquisition as acquisition


class MintPageCreationTests(unittest.IsolatedAsyncioTestCase):
    finish = acquisition.PageAcquisitionTests.finish
    block = acquisition.PageAcquisitionTests.block
    start = acquisition.PageAcquisitionTests.start
    assert_waiting = acquisition.PageAcquisitionTests.assert_waiting

    def setUp(self):
        acquisition.PageAcquisitionTests.setUp(self)
        forge = patch.object(self.module, "_cf", None)
        forge.start()
        self.addCleanup(forge.stop)

    def account(self, name="a"):
        acc = acquisition.PageAcquisitionTests.account(self, name)
        rates = acc.lifecycle.rates
        rates.policy = replace(rates.policy, mint_gap=0)
        async def evaluate(expression):
            if expression == self.module.WARM_JS:
                return True
            if expression == self.module.MINT_JS:
                return "fixture-token"
            raise AssertionError("unexpected evaluation")
        acc.ctx.next_page.evaluate = AsyncMock(side_effect=evaluate)
        return acc

    async def mint(self, name="a"):
        async with self.host._use_account(name) as acc:
            return await self.host._mint(acc, acc.page, name)

    def command(self, name="a"):
        task = self.start(self.mint(name))
        key = len(self.waiters)
        self.host._command_tasks[key] = task
        task.add_done_callback(lambda _: self.host._command_tasks.pop(key, None))
        return task

    def assert_idle(self, acc):
        acquisition.PageAcquisitionTests.assert_idle(self, acc)
        self.assertFalse(acc.mint_lock.locked())
        self.assertFalse(acc.lifecycle.rates.state["pending_mint"])

    async def test_cancelled_creation_closes_late_page_without_warming(self):
        acc = self.account()
        page = acc.ctx.next_page
        creation = self.block(acc.ctx.creation)
        request = self.command()
        await asyncio.wait_for(creation.entered.wait(), 1)
        request.cancel()
        await self.assert_waiting(request)
        creation.release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(request, 1)
        self.assertTrue(page.is_closed())
        self.assertEqual(page.disposal.calls, 1)
        self.assertEqual(page.gotos, [])
        self.assertEqual(page.waits, [])
        page.evaluate.assert_not_awaited()
        self.assertIsNone(acc.mint_page)
        self.assertEqual(acc.size, 1)
        self.assertEqual(acc.pool.qsize(), 1)
        self.assertFalse(acc.borrowed)
        self.assert_idle(acc)

    async def test_repeated_cancellation_retains_close_and_mint_ownership(self):
        acc, other = self.account(), self.account("b")
        creation = self.block(acc.ctx.creation)
        disposal = self.block(acc.ctx.next_page.disposal)
        request = self.command()
        await asyncio.wait_for(creation.entered.wait(), 1)
        request.cancel()
        await self.assert_waiting(request)
        creation.release.set()
        await asyncio.wait_for(disposal.entered.wait(), 1)
        entered = asyncio.Event()
        async def next_owner():
            async with self.host._mint_gate(acc):
                entered.set()
        waiting = self.start(next_owner())
        for _ in range(2):
            request.cancel()
            await self.assert_waiting(request)
        await self.assert_waiting(waiting)
        self.assertFalse(entered.is_set())
        self.assertTrue(acc.mint_lock.locked())
        self.assertTrue(acc.lifecycle.rates.state["pending_mint"])
        self.assertEqual(self.host._reserved_bytes, 64 * 1024**2)
        self.assertEqual(await asyncio.wait_for(self.command("b"), 1),
                         ("fixture-token", None))
        self.assertIs(other.mint_page, other.ctx.next_page)
        self.assertFalse(await self.host._close_account("a"))
        disposal.release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(request, 1)
        await asyncio.wait_for(waiting, 1)
        self.assertTrue(entered.is_set())
        self.assertTrue(acc.ctx.next_page.is_closed())
        acc.ctx.next_page.evaluate.assert_not_awaited()
        self.assert_idle(acc)
        self.assert_idle(other)

    async def test_late_creation_failure_keeps_cancellation_without_close(self):
        acc = self.account()
        creation = self.block(acc.ctx.creation)
        creation.failure = RuntimeError("fixture creation failure")
        request = self.command()
        await asyncio.wait_for(creation.entered.wait(), 1)
        request.cancel()
        creation.release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(request, 1)
        self.assertEqual(acc.ctx.next_page.disposal.calls, 0)
        acc.ctx.next_page.evaluate.assert_not_awaited()
        self.assertIsNone(acc.mint_page)
        self.assert_idle(acc)

    async def test_failed_late_close_is_reported_without_claiming_cleanup(self):
        acc = self.account()
        page = acc.ctx.next_page
        creation = self.block(acc.ctx.creation)
        error = RuntimeError("fixture page close failed")
        page.disposal.failure = error
        request = self.command()
        await asyncio.wait_for(creation.entered.wait(), 1)
        request.cancel()
        await self.assert_waiting(request)
        creation.release.set()
        with self.assertRaises(RuntimeError) as raised:
            await asyncio.wait_for(request, 1)
        self.assertIs(raised.exception, error)
        self.assertFalse(page.is_closed())
        self.assertEqual(page.disposal.calls, 1)
        page.evaluate.assert_not_awaited()
        self.assertIsNone(acc.mint_page)
        self.assert_idle(acc)

    async def test_normal_mint_prepares_once_and_reuses_its_published_page(self):
        acc = self.account()
        for _ in range(2):
            self.assertEqual(await asyncio.wait_for(self.command(), 1),
                             ("fixture-token", None))
        page = acc.ctx.next_page
        self.assertIs(acc.mint_page, page)
        self.assertEqual(acc.ctx.creation.calls, 1)
        self.assertEqual(page.evaluate.await_args_list,
                         [call(self.module.WARM_JS), call(self.module.MINT_JS),
                          call(self.module.MINT_JS)])
        self.assertEqual(page.gotos, [(self.module.LOGIN_URL,
            {"wait_until": "domcontentloaded", "timeout": 60000})])
        self.assertEqual(page.waits, [800])
        self.assertEqual(page.disposal.calls, 0)
        self.assertEqual(len(acc.tasks), 1)
        self.assert_idle(acc)

    async def test_normal_creation_and_preparation_errors_are_preserved(self):
        for stage in ("creation", "navigation", "warming"):
            with self.subTest(stage=stage):
                acc = self.account(stage)
                page = acc.ctx.next_page
                error = RuntimeError("fixture " + stage)
                if stage == "creation":
                    acc.ctx.creation.failure = error
                elif stage == "navigation":
                    page.navigation.failure = error
                else:
                    page.evaluate.side_effect = error
                with self.assertRaises(RuntimeError) as raised:
                    await asyncio.wait_for(self.command(stage), 1)
                self.assertIs(raised.exception, error)
                self.assertEqual(page.disposal.calls, 0 if stage == "creation" else 1)
                self.assertIsNone(acc.mint_page)
                self.assertNotIn(call(self.module.MINT_JS), page.evaluate.await_args_list)
                self.assert_idle(acc)

    async def test_shutdown_waits_for_late_page_close_after_cancellation(self):
        acc = self.account()
        creation = self.block(acc.ctx.creation)
        disposal = self.block(acc.ctx.next_page.disposal)
        request = self.command()
        await asyncio.wait_for(creation.entered.wait(), 1)
        request.cancel()
        creation.release.set()
        await asyncio.wait_for(disposal.entered.wait(), 1)
        closing = self.start(self.host.close())
        await self.assert_waiting(closing)
        request.cancel()
        await self.assert_waiting(request)
        self.assertEqual(acc.browser.calls, 0)
        disposal.release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(request, 1)
        await asyncio.wait_for(closing, 1)
        self.assertTrue(acc.ctx.next_page.is_closed())
        self.assertTrue(acc.browser.acknowledged.is_set())
        self.assertEqual(acc.lifecycle.phase, "absent")
        acc.ctx.next_page.evaluate.assert_not_awaited()
        self.assert_idle(acc)
