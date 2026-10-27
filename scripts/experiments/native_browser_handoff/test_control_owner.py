"""Real Host ownership and shutdown, with inert browser and request ports."""
import asyncio
from dataclasses import replace
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "runtime"))
sys.path.insert(0, str(ROOT / "runtime/tests"))
from framerize_host import app
from framerize_host.browser import driver
from test_lifecycle import FakeBrowser
from test_control_mint import Page, EXPECTED
from control_owner import Consumed, ControlHost, ControlIdentity, ObservationRefused


class OwnerTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        override = patch.multiple(app, ROOT=directory.name, emit=lambda *a, **k: None)
        override.start()
        self.addCleanup(override.stop)
        protocol = patch.object(driver, "verify_close_protocol", lambda: None)
        protocol.start()
        self.addCleanup(protocol.stop)
        self.host, self.life, self.browser = ControlHost(), app.AccountLifecycle(), FakeBrowser()
        self.life.browser, self.life.phase, self.life.incarnation = self.browser, "resident", 1
        self.life.close_receipt = app.BrowserCloseReceipt(self.browser)
        self.page = Page()
        self.page.context.browser = self.browser
        self.acc = app.Account(None, self.page.context, self.page, self.life)
        self.acc.browser, self.acc.session_cookie = self.browser, ("session", EXPECTED.session)
        device = app.Device(EXPECTED.device)
        device.page, device.ctx = self.page, self.page.context
        self.acc.dev_objs[EXPECTED.device] = device
        self.host.accounts["fixture"] = self.acc
        self.host.lifecycles["fixture"] = self.life
        self.identity = ControlIdentity("fixture", self.host.generation, 1, 0)
        self.entered, self.release = asyncio.Event(), asyncio.Event()
        self.consumer_calls, self.extra_tasks = 0, []

    async def consume(self, minted, cancelled, deadline):
        self.consumer_calls += 1
        self.assertEqual(minted.token, "fixture-token")
        return Consumed("completed", True)

    async def start(self, consume=None, deadline=None, identity=None):
        return await self.host.run_control(identity or self.identity, EXPECTED,
            consume or self.consume, deadline or time.monotonic() + 5)

    async def asyncTearDown(self):
        self.release.set()
        self.browser.release.set()
        await asyncio.gather(*self.extra_tasks, return_exceptions=True)
        await self.host.close()

    async def test_success_joins_one_consumer_and_rejects_control_reuse(self):
        result = await self.start()
        self.assertEqual(result, Consumed("completed", True))
        self.assertEqual(self.consumer_calls, 1)
        self.assertFalse(self.host._control_busy)
        self.assertFalse(self.host._control_fenced)
        self.assertFalse(self.life.activity.pending or self.life.activity.activities)
        with self.assertRaises(ObservationRefused):
            await self.start()
        self.assertEqual(self.page.calls.count("evaluate"), 1)

    async def test_stale_identity_refuses_before_observer_or_reservation(self):
        for identity in (replace(self.identity, host_generation="changed"),
                replace(self.identity, incarnation=2), replace(self.identity, proxy_revision=1)):
            with self.subTest(identity=repr(identity)):
                with self.assertRaises(ObservationRefused):
                    await self.start(identity=identity)
                self.assertFalse(self.host._control_used)
                self.assertEqual(self.page.calls, [])

    async def test_absent_selected_device_has_no_page_fallback(self):
        self.acc.dev_objs.clear()
        with self.assertRaises(ObservationRefused):
            await self.start()
        self.assertEqual(self.page.calls, [])
        self.assertFalse(self.host._control_used)

    async def test_existing_raw_rpc_refuses_control_until_actual_completion(self):
        async def held():
            self.entered.set()
            await self.release.wait()
        task = asyncio.create_task(self.host._rpc("fixture", held))
        self.extra_tasks.append(task)
        await asyncio.wait_for(self.entered.wait(), 1)
        with self.assertRaises(ObservationRefused):
            await self.start()
        self.assertFalse(self.host._control_used)
        self.release.set()
        await task
        self.assertEqual(await self.start(), Consumed("completed", True))

    async def test_running_background_work_refuses_control_without_cancelling_it(self):
        task = asyncio.create_task(self.release.wait())
        self.extra_tasks.append(task)
        self.acc.tasks.add(task)
        with self.assertRaises(ObservationRefused):
            await self.start()
        self.assertFalse(task.done())
        self.assertFalse(self.host._control_used)
        self.assertEqual(self.page.calls, [])

    async def test_ordinary_commands_rpcs_and_second_control_cannot_enter(self):
        async def consume(*args):
            self.entered.set()
            await self.release.wait()
            return Consumed("completed", True)
        task = asyncio.create_task(self.start(consume))
        self.extra_tasks.append(task)
        await asyncio.wait_for(self.entered.wait(), 1)
        with self.assertRaises(ObservationRefused):
            await self.host.handle({"cmd": "browser_status", "account": "fixture"})
        with self.assertRaises(ObservationRefused):
            await self.host._rpc("fixture", self.page.evaluate, "", {})
        with self.assertRaises(ObservationRefused):
            async with self.host._activity("fixture"):
                self.fail("ordinary activity entered")
        with self.assertRaises(ObservationRefused):
            await self.start()
        self.release.set()
        self.assertEqual(await task, Consumed("completed", True))
        self.assertEqual(self.page.calls.count("evaluate"), 1)

    async def test_rpc_deadline_retains_actual_work_until_it_joins(self):
        self.page.evaluate = self.hold_evaluate
        task = asyncio.create_task(self.start(deadline=time.monotonic() + 0.2))
        self.extra_tasks.append(task)
        await asyncio.wait_for(self.entered.wait(), 1)
        await asyncio.wait_for(self.host._control_cancelled.wait(), 1)
        self.assertFalse(task.done())
        self.assertEqual(len(self.life.activity.pending), 1)
        self.release.set()
        with self.assertRaises(ObservationRefused):
            await task
        self.assertTrue(self.host._control_fenced)
        self.assertFalse(self.life.activity.pending or self.life.activity.activities)
        self.assertEqual(self.consumer_calls, 0)

    async def test_consumer_deadline_waits_for_its_actual_completion(self):
        async def consume(minted, cancelled, deadline):
            self.entered.set()
            await self.release.wait()
            return Consumed("completed", True)
        task = asyncio.create_task(self.start(consume, time.monotonic() + 0.2))
        self.extra_tasks.append(task)
        await asyncio.wait_for(self.entered.wait(), 1)
        await asyncio.wait_for(self.host._control_cancelled.wait(), 1)
        self.assertFalse(task.done())
        self.release.set()
        with self.assertRaises(ObservationRefused):
            await task
        self.assertTrue(self.host._control_fenced)

    async def hold_evaluate(self, *args):
        self.page.calls.append("evaluate")
        self.entered.set()
        await self.release.wait()
        return self.page.result

    async def test_cancelled_waiter_retains_raw_rpc_then_fenced_shutdown_closes(self):
        self.page.evaluate = self.hold_evaluate
        task = asyncio.create_task(self.start())
        self.extra_tasks.append(task)
        await asyncio.wait_for(self.entered.wait(), 1)
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertEqual(len(self.life.activity.pending), 1)
        self.assertTrue(self.host._control_busy and self.host._control_fenced)
        self.assertFalse(self.host._control_task.done())
        closing = asyncio.create_task(self.host.close())
        self.extra_tasks.append(closing)
        await asyncio.sleep(0)
        self.assertEqual(self.browser.calls, 0)
        self.release.set()
        await asyncio.wait_for(closing, 1)
        self.assertFalse(self.host._control_busy)
        self.assertTrue(self.host._control_fenced)
        self.assertFalse(self.life.activity.pending or self.life.activity.activities)
        self.assertFalse(self.browser.connected)
        self.assertEqual(self.consumer_calls, 0)

    async def test_shutdown_waits_for_consumer_and_preserves_refusal_fence(self):
        async def consume(minted, cancelled, deadline):
            self.entered.set()
            await cancelled.wait()
            await self.release.wait()
            return Consumed("completed", True)
        task = asyncio.create_task(self.start(consume))
        self.extra_tasks.append(task)
        await asyncio.wait_for(self.entered.wait(), 1)
        closing = asyncio.create_task(self.host.close())
        self.extra_tasks.append(closing)
        await asyncio.wait_for(self.host._control_cancelled.wait(), 1)
        self.assertFalse(closing.done())
        self.assertTrue(self.host._control_busy)
        self.assertEqual(self.browser.calls, 0)
        self.release.set()
        with self.assertRaises(ObservationRefused):
            await task
        await asyncio.wait_for(closing, 1)
        self.assertTrue(self.host._control_fenced)
        self.assertFalse(self.browser.connected)
        with self.assertRaises(ObservationRefused):
            await self.host.handle({"cmd": "browser_status", "account": "fixture"})

    async def test_unconfirmed_consumer_fences_admission_but_allows_owned_close(self):
        async def consume(*args):
            return Consumed("unconfirmed", False)
        with self.assertRaises(ObservationRefused):
            await self.start(consume)
        self.assertTrue(self.host._control_fenced)
        with self.assertRaises(ObservationRefused):
            await self.host._close_account("fixture")
        self.assertTrue(self.browser.connected)
        await self.host.close()
        self.assertFalse(self.browser.connected)
        self.assertTrue(self.host._control_fenced)

    async def test_changed_host_generation_never_reaches_mint_or_consumer(self):
        self.page.after_cookie = lambda url: setattr(self.host, "generation", "changed")
        with self.assertRaises(ObservationRefused):
            await self.start()
        self.assertNotIn("evaluate", self.page.calls)
        self.assertEqual(self.consumer_calls, 0)
        self.assertTrue(self.host._control_fenced)


    async def test_changed_proxy_revision_refuses_even_a_settled_consumer(self):
        async def consume(*args):
            self.host.proxy_revisions["fixture"] = 1
            return Consumed("completed", True)
        with self.assertRaises(ObservationRefused):
            await self.start(consume)
        self.assertTrue(self.host._control_fenced)

    async def test_consumer_exception_is_payload_free_and_cleanup_still_runs(self):
        async def consume(*args):
            raise RuntimeError("fixture-private-detail")
        with self.assertRaises(ObservationRefused) as caught:
            await self.start(consume)
        self.assertEqual(str(caught.exception), "browser mint observation refused")
        self.assertTrue(self.host._control_fenced)
        await self.host.close()
        self.assertFalse(self.browser.connected)

    async def test_nonboolean_cleanup_report_is_not_accepted(self):
        async def consume(*args):
            return Consumed("completed", 1)
        with self.assertRaises(ObservationRefused):
            await self.start(consume)
        self.assertTrue(self.host._control_fenced)


if __name__ == "__main__":
    unittest.main()
