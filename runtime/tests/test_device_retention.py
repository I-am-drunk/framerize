"""Idle device eviction against real host ownership and inert browser ports."""
import asyncio
from collections import deque
from copy import deepcopy
import unittest
from unittest.mock import AsyncMock, Mock, call, patch

import test_device_context_creation as device_fixture
import test_page_acquisition as acquisition
from framerize_host.accounts.device_retention import trim_idle_device_contexts


class DeviceRetentionTests(unittest.IsolatedAsyncioTestCase):
    setUp = device_fixture.DeviceContextCreationTests.setUp
    finish = acquisition.PageAcquisitionTests.finish
    block = acquisition.PageAcquisitionTests.block
    start = acquisition.PageAcquisitionTests.start
    assert_waiting = acquisition.PageAcquisitionTests.assert_waiting
    assert_idle = acquisition.PageAcquisitionTests.assert_idle
    prepare = device_fixture.DeviceContextCreationTests.prepare
    command = device_fixture.DeviceContextCreationTests.command

    def account(self, name="a"):
        acc = device_fixture.DeviceContextCreationTests.account(self, name)
        create = acc.browser.new_context

        async def create_context():
            context = await create()
            close = context.close

            async def acknowledged_close():
                await close()
                if context in acc.browser.contexts:
                    acc.browser.contexts.remove(context)

            context.close = acknowledged_close
            return context

        acc.browser.new_context = AsyncMock(side_effect=create_context)
        return acc

    async def populated(self, name="a", count=4):
        acc = self.account(name)
        names = ["primary"] + [f"device-{n}" for n in range(1, count)]
        acc.devices[:] = names
        acc.primary_dev = "primary"
        for index, device_id in enumerate(names):
            device = await asyncio.wait_for(self.command(name, device_id), 1)
            device.last_used = float(index)
        return acc

    async def trim(self, acc, name="a", keep=2):
        return await trim_idle_device_contexts(self.host, name, acc, keep)

    async def test_preserves_primary_recent_identity_and_history(self):
        acc = await self.populated()
        acc.lifecycle.rates.state["requests"]["device-1"] = deque([1.0, 2.0])
        acc.lifecycle.rates.state["cooling"]["device-1"] = 100.0
        before = deepcopy(acc.lifecycle.rates.state)
        cookie, devices = acc.session_cookie, dict(acc.dev_objs)
        contexts = {key: d.ctx for key, d in devices.items()}
        self.assertEqual(await self.trim(acc), 2)
        self.assertEqual(acc.lifecycle.rates.state, before)
        self.assertEqual(acc.session_cookie, cookie)
        self.assertEqual(acc.dev_objs, devices)
        for key in ("primary", "device-3"):
            self.assertIs(devices[key].ctx, contexts[key])
            self.assertFalse(contexts[key].closed)
        for key in ("device-1", "device-2"):
            self.assertIsNone(devices[key].ctx)
            self.assertIsNone(devices[key].page)
            self.assertFalse(devices[key].warm)
            self.assertTrue(contexts[key].closed)
            self.assertNotIn(contexts[key], acc.browser.contexts)
        self.assert_idle(acc)

    async def test_each_pass_closes_at_most_two(self):
        acc = await self.populated(count=6)
        self.assertEqual(await self.trim(acc), 2)
        self.assertEqual(sum(d.ctx is not None for d in acc.dev_objs.values()), 4)
        self.assertEqual(await self.trim(acc), 2)
        self.assertEqual(sum(d.ctx is not None for d in acc.dev_objs.values()), 2)
        self.assertEqual(await self.trim(acc), 0)

    async def test_missing_primary_keeps_most_recent_devices(self):
        acc = await self.populated()
        acc.primary_dev = None
        self.assertEqual(await self.trim(acc), 2)
        self.assertEqual([d.id for d in acc.dev_objs.values() if d.ctx is not None],
                         ["device-2", "device-3"])

    async def test_invalid_retention_is_rejected_without_close(self):
        acc = await self.populated()
        for keep in (0, -1, True, 1.5):
            with self.assertRaises(ValueError):
                await self.trim(acc, keep=keep)
        self.assertTrue(all(d.ctx is not None for d in acc.dev_objs.values()))

    async def test_foreground_and_background_activity_prevent_close(self):
        acc = await self.populated()
        async with self.host._use_account("a"):
            self.assertEqual(await self.trim(acc), 0)
        async with self.host._activity("a"):
            self.assertEqual(acc.active_requests, 0)
            self.assertEqual(await self.trim(acc), 0)
        self.assertTrue(all(d.ctx.disposal.calls == 0 for d in acc.dev_objs.values()))
        self.assertEqual(await self.trim(acc), 2)

    async def test_busy_locks_and_account_flags_prevent_close(self):
        acc = await self.populated()
        locks = [acc.lock, acc.mint_lock, acc.cookie_lock,
                 acc.lifecycle.creation_lock, acc.dev_objs["device-1"].lock]
        for lock in locks:
            async with lock:
                self.assertEqual(await self.trim(acc), 0)
        fields = [(acc, "active_requests", 1), (acc, "closing", True),
                  (acc.lifecycle, "active_commands", 1),
                  (acc.lifecycle, "phase", "unconfirmed")]
        for owner, name, value in fields:
            with self.subTest(field=name):
                previous = getattr(owner, name)
                setattr(owner, name, value)
                try:
                    self.assertEqual(await self.trim(acc), 0)
                finally:
                    setattr(owner, name, previous)
        self.assertTrue(all(d.ctx.disposal.calls == 0 for d in acc.dev_objs.values()))

    async def test_borrowed_page_and_active_token_prevent_close(self):
        acc = await self.populated()
        acc.borrowed.add(acc.page)
        self.assertEqual(await self.trim(acc), 0)
        acc.borrowed.clear()
        acc.token_pool["fixture"] = {"in_use": True}
        self.assertEqual(await self.trim(acc), 0)
        acc.token_pool["fixture"]["in_use"] = False
        self.assertEqual(await self.trim(acc), 2)

    async def test_cancelled_waiter_keeps_raw_rpc_busy(self):
        acc = await self.populated()
        gate = self.block(acquisition.Call())
        waiter = self.start(self.host._rpc("a", gate))
        await asyncio.wait_for(gate.entered.wait(), 1)
        waiter.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await waiter
        self.assertTrue(acc.lifecycle.activity.pending)
        self.assertEqual(await self.trim(acc), 0)
        gate.release.set()
        await asyncio.gather(*acc.lifecycle.activity.pending)
        self.assertEqual(await self.trim(acc), 2)

    async def test_new_device_use_waits_for_close_and_recreates_same_identity(self):
        acc = await self.populated()
        device = acc.dev_objs["device-1"]
        context = device.ctx
        gate = self.block(context.disposal)
        trimming = self.start(self.trim(acc))
        await asyncio.wait_for(gate.entered.wait(), 1)
        request = self.command(device="device-1")
        await self.assert_waiting(request)
        self.assertTrue(device.lock.locked())
        self.assertEqual(acc.browser.new_context.await_count, 4)
        gate.release.set()
        self.assertEqual(await asyncio.wait_for(trimming, 1), 1)
        reused = await asyncio.wait_for(request, 1)
        self.assertIs(reused, device)
        self.assertIsNot(reused.ctx, context)
        self.assertTrue(context.closed)
        self.assertTrue(reused.warm)
        self.assertEqual(reused.id, "device-1")
        self.assertEqual(acc.browser.new_context.await_count, 5)
        self.assert_idle(acc)

    async def test_repeated_cancellation_keeps_lock_until_actual_close(self):
        acc = await self.populated()
        device, other = acc.dev_objs["device-1"], self.account("b")
        context = device.ctx
        gate = self.block(context.disposal)
        trimming = self.start(self.trim(acc))
        await asyncio.wait_for(gate.entered.wait(), 1)
        trimming.cancel("cancelled trimming")
        await self.assert_waiting(trimming)
        trimming.cancel("cancelled again")
        await self.assert_waiting(trimming)
        self.assertTrue(device.lock.locked())
        self.assertIs(device.ctx, context)
        self.assertTrue(acc.lifecycle.activity.pending)
        foreign = await asyncio.wait_for(self.command("b", "foreign"), 1)
        self.assertIs(foreign.ctx, other.browser.created_contexts[0])
        request = self.command(device="device-1")
        await self.assert_waiting(request)
        gate.release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(trimming, 1)
        reused = await asyncio.wait_for(request, 1)
        self.assertTrue(context.closed)
        self.assertIs(reused, device)
        self.assertIsNot(reused.ctx, context)
        self.assertFalse(acc.closing)
        self.assert_idle(acc)

    async def test_failed_close_retains_handles_and_fences_only_its_account(self):
        acc, other = await self.populated(), await self.populated("b")
        device = acc.dev_objs["device-1"]
        context, page = device.ctx, device.page
        context.disposal.failure = RuntimeError("fixture close failed; token=private-fixture-value")
        self.assertEqual(await self.trim(acc), 0)
        self.assertIs(device.ctx, context)
        self.assertIs(device.page, page)
        self.assertIn(context, acc.browser.contexts)
        self.assertTrue(acc.closing)
        self.assertEqual(acc.lifecycle.phase, "unconfirmed")
        self.events.assert_called_once_with("browser.cleanup", level="warn",
            outcome="unconfirmed", error="idle-device-context-close-failed",
            cause="RuntimeError: fixture close failed; token=<redacted>")
        with self.assertRaisesRegex(RuntimeError, "fixture close failed; token=<redacted>"):
            await self.host._account("a")
        self.assertEqual(await self.trim(other, "b"), 2)

    async def test_returned_close_without_closure_is_not_success(self):
        acc = await self.populated()
        device, context = acc.dev_objs["device-1"], acc.dev_objs["device-1"].ctx
        context.close = AsyncMock()
        self.assertEqual(await self.trim(acc), 0)
        self.assertIs(device.ctx, context)
        self.assertIsNotNone(device.page)
        self.assertEqual(acc.lifecycle.phase, "unconfirmed")
        self.assertTrue(acc.closing)

    async def test_activity_starting_before_close_dispatch_prevents_effects(self):
        acc = await self.populated()
        original = self.host._rpc

        async def overlapping(account, call, *args, **kwargs):
            async with self.host._activity(account):
                return await original(account, call, *args, **kwargs)

        with patch.object(self.host, "_rpc", overlapping):
            self.assertEqual(await self.trim(acc), 0)
        self.assertTrue(all(d.ctx.disposal.calls == 0 for d in acc.dev_objs.values()))
        self.assertFalse(acc.closing)
        self.assert_idle(acc)

    async def test_stale_account_owner_cannot_close_replacement(self):
        acc = await self.populated()
        replacement = self.account("replacement")
        self.host.accounts["a"] = replacement
        try:
            self.assertEqual(await self.trim(acc), 0)
            self.assertTrue(all(d.ctx.disposal.calls == 0 for d in acc.dev_objs.values()))
        finally:
            self.host.accounts["a"] = acc

    async def test_cancelled_failed_close_still_fences_and_retains_handles(self):
        acc = await self.populated()
        device = acc.dev_objs["device-1"]
        context, page = device.ctx, device.page
        gate = self.block(context.disposal)
        gate.failure = RuntimeError("fixture close failed after waiter loss")
        trimming = self.start(self.trim(acc))
        await asyncio.wait_for(gate.entered.wait(), 1)
        trimming.cancel()
        await self.assert_waiting(trimming)
        gate.release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(trimming, 1)
        self.assertIs(device.ctx, context)
        self.assertIs(device.page, page)
        self.assertTrue(acc.closing)
        self.assertEqual(acc.lifecycle.phase, "unconfirmed")
        self.assertFalse(device.lock.locked())
        self.assertFalse(acc.lifecycle.activity.pending)

    async def test_browser_disconnect_cannot_confirm_context_cleanup(self):
        acc = await self.populated()
        device = acc.dev_objs["device-1"]
        context, page, close = device.ctx, device.page, device.ctx.close

        async def disconnecting_close():
            await close()
            acc.browser.connected = False

        context.close = disconnecting_close
        self.assertEqual(await self.trim(acc), 0)
        self.assertTrue(context.closed)
        self.assertIs(device.ctx, context)
        self.assertIs(device.page, page)
        self.assertTrue(acc.closing)
        self.assertEqual(acc.lifecycle.phase, "unconfirmed")

    async def test_monitor_trims_and_preserves_other_maintenance(self):
        acc = await self.populated()
        snapshot, emitted = {}, Mock()
        before = deepcopy(acc.lifecycle.rates.state)
        with patch.object(self.module, "memory_snapshot", return_value=snapshot), \
             patch.object(self.module, "emit", emitted), \
             patch.object(self.host, "_reclaim_idle_memory",
                          wraps=self.host._reclaim_idle_memory) as gc, \
             patch.object(self.host, "_trim_idle_heap",
                          wraps=self.host._trim_idle_heap) as heap:
            await asyncio.wait_for(self.host._monitor_once(), 1)
        gc.assert_awaited_once_with(snapshot)
        heap.assert_called_once_with(snapshot)
        self.assertEqual(emitted.call_args_list, [
            call("memory.snapshot", accounts=1, active_requests=0,
                 background_tasks=0, pages=5),
            call("memory.device_trim", closed=2, retained=2)])
        self.assertEqual([d.id for d in acc.dev_objs.values() if d.ctx is not None],
                         ["primary", "device-3"])
        self.assertEqual(acc.lifecycle.rates.state, before)
        self.assert_idle(acc)

    async def test_monitor_reclaims_every_idle_account_in_one_pass(self):
        accounts = [await self.populated(name) for name in ("a", "b", "c")]
        await asyncio.wait_for(self.host._monitor_once(), 1)
        self.assertEqual([sum(d.ctx is not None for d in acc.dev_objs.values())
                          for acc in accounts], [2, 2, 2])

    async def test_monitor_reclaims_unused_devices_during_account_activity(self):
        first, second = await self.populated(), await self.populated("b")
        async with self.host._activity("a"):
            await asyncio.wait_for(self.host._monitor_once(), 1)
            self.assertEqual(sum(d.ctx is not None for d in first.dev_objs.values()), 2)
            self.assertEqual(sum(d.ctx is not None for d in second.dev_objs.values()), 2)
        await asyncio.wait_for(self.host._monitor_once(), 1)
        self.assertEqual(sum(d.ctx is not None for d in first.dev_objs.values()), 2)

    async def test_monitor_regrowing_first_account_cannot_starve_idle_peer(self):
        first, second = await self.populated(), await self.populated("b")
        for _ in range(3):
            for device_id in ("device-1", "device-2"):
                await asyncio.wait_for(self.command(device=device_id), 1)
            await asyncio.wait_for(self.host._monitor_once(), 1)
        self.assertEqual(sum(d.ctx is not None for d in second.dev_objs.values()), 2)
        self.assertIsNotNone(first.dev_objs["primary"].ctx)

    async def test_monitor_preserves_existing_expired_account_cleanup(self):
        acc = await self.populated()
        self.host.idle_timeout = 1
        acc.last_used = 0
        with patch.object(self.host, "_reclaim_idle_memory",
                          wraps=self.host._reclaim_idle_memory) as gc, \
             patch.object(self.host, "_trim_idle_heap",
                          wraps=self.host._trim_idle_heap) as heap:
            await asyncio.wait_for(self.host._monitor_once(), 1)
        self.assertNotIn("a", self.host.accounts)
        self.assertEqual(acc.lifecycle.phase, "absent")
        self.assertTrue(acc.browser.acknowledged.is_set())
        self.assertEqual(gc.await_count, 1)
        self.assertEqual(heap.call_count, 1)
