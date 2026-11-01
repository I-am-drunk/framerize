"""Fresh request-token ownership through inert browser wrapper calls."""
import asyncio
from copy import deepcopy
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, Mock, call, patch

import test_device_context_creation as creation
import test_heartbeat_retention as heartbeat
import test_page_acquisition as acquisition


class FreshTokenTests(unittest.IsolatedAsyncioTestCase):
    async def test_exhausted_devices_do_not_create_another_identity(self):
        acc, _ = await self.ready()
        acc.devices[:] = ["device-a"]
        acc.primary_dev = "device-a"
        self.host._cool_device(acc, "device-a", "a")
        before = deepcopy(acc.lifecycle.rates.state)
        self.assertIsNone(self.host._pick_device(acc, "a"))
        after = acc.lifecycle.rates.state
        for field in ("devices", "reserve", "reserve_used", "cooling", "requests"):
            self.assertEqual(after[field], before[field])
        reason = acc.lifecycle.rates.unavailable_reason()
        self.assertIn("HTTP 403", reason)
        self.assertIn("security checks", reason)

    setUp = creation.DeviceContextCreationTests.setUp
    finish = creation.DeviceContextCreationTests.finish
    account = creation.DeviceContextCreationTests.account
    prepare = creation.DeviceContextCreationTests.prepare
    start = acquisition.PageAcquisitionTests.start
    block = acquisition.PageAcquisitionTests.block
    assert_waiting = acquisition.PageAcquisitionTests.assert_waiting
    cycle = heartbeat.HeartbeatRetentionTests.cycle

    async def ready(self, name="a", device="device-a"):
        acc = self.account(name)
        prepared = await self.prepare(name, device)
        prepared.page.evaluate.reset_mock()
        return acc, prepared

    async def acquire(self, name="a", device="device-a"):
        async with self.host._use_account(name) as acc:
            return await self.host._token_acquire(acc, acc.page, name, device)

    async def test_sequential_requests_mint_fresh_tokens(self):
        acc, device = await self.ready()
        device.page.evaluate.side_effect = ["first-token", "second-token"]
        first = await self.acquire()
        self.host._token_release(acc, first[0])
        second = await self.acquire()
        self.assertEqual((first[1], second[1]), ("first-token", "second-token"))
        self.assertEqual((first[2], second[2]), ("device-a", "device-a"))
        self.assertNotEqual(first[0], second[0])
        self.assertEqual(device.page.evaluate.await_args_list,
                         [call(self.module.MINT_JS), call(self.module.MINT_JS)])
        self.host._token_release(acc, second[0])

    async def test_cached_token_is_never_reused(self):
        acc, device = await self.ready()
        stamp = self.module.time.monotonic()
        acc.token_pool["old-slot"] = {
            "token": "cached-token", "born": stamp, "ts": stamp,
            "uses": 1, "in_use": False, "dev": "device-a"}
        device.page.evaluate.return_value = "new-wrapper-token"
        slot, token, used = await self.acquire()
        self.assertEqual((token, used), ("new-wrapper-token", "device-a"))
        self.assertNotEqual(slot, "old-slot")
        device.page.evaluate.assert_awaited_once_with(self.module.MINT_JS)
        self.host._token_release(acc, slot)

    async def test_simultaneous_requests_mint_separately_under_the_gate(self):
        acc, device = await self.ready()
        gate = self.block(acquisition.Call())
        calls = []
        async def mint(expression):
            calls.append(expression)
            index = len(calls)
            if index == 1:
                await gate()
            return f"wrapper-token-{index}"
        device.page.evaluate.side_effect = mint
        first = self.start(self.acquire())
        await asyncio.wait_for(gate.entered.wait(), 1)
        second = self.start(self.acquire())
        await self.assert_waiting(second)
        self.assertEqual(calls, [self.module.MINT_JS])
        self.assertEqual(acc.token_pool, {})
        gate.release.set()
        results = await asyncio.wait_for(asyncio.gather(first, second), 1)
        for slot, _, _ in results:
            self.host._token_release(acc, slot)
        self.assertEqual([item[1] for item in results],
                         ["wrapper-token-1", "wrapper-token-2"])
        self.assertEqual([item[2] for item in results], ["device-a"] * 2)
        self.assertNotEqual(results[0][0], results[1][0])
        self.assertEqual(calls, [self.module.MINT_JS] * 2)

    async def test_release_discards_metadata_and_retains_no_token(self):
        acc, device = await self.ready()
        device.page.evaluate.return_value = "private-wrapper-result"
        slot, token, _ = await self.acquire()
        self.assertIn(slot, acc.token_pool)
        self.assertTrue(acc.token_pool[slot]["in_use"])
        self.assertNotIn(token, repr(acc.token_pool))
        self.host._token_release(acc, slot)
        self.assertEqual(acc.token_pool, {})
        self.host._token_release(acc, slot)
        self.host._token_release(acc, None)
        self.assertEqual(acc.token_pool, {})

    async def test_cancelled_acquisition_never_publishes_the_late_token(self):
        acc, device = await self.ready()
        gate = self.block(acquisition.Call())
        async def mint(_expression):
            await gate()
            return "late-wrapper-token"
        device.page.evaluate.side_effect = mint
        pending = self.start(self.acquire())
        await asyncio.wait_for(gate.entered.wait(), 1)
        pending.cancel("cancelled token acquisition")
        await self.assert_waiting(pending)
        self.assertEqual(acc.token_pool, {})
        self.assertTrue(acc.mint_lock.locked())
        gate.release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(pending, 1)
        self.assertEqual(acc.token_pool, {})
        self.assertFalse(acc.mint_lock.locked())

    async def test_acquisition_age_includes_wrapper_wait_and_expires_at_120s(self):
        acc, device = await self.ready()
        stamp = [1000.0]
        clock = SimpleNamespace(monotonic=lambda: stamp[0])
        for elapsed, usable in ((119.0, True), (120.0, False), (121.0, False)):
            with self.subTest(elapsed=elapsed):
                stamp[0] = 1000.0
                async def mint(_expression):
                    stamp[0] += elapsed
                    return "timed-wrapper-token"
                device.page.evaluate.side_effect = mint
                with patch.object(self.module, "time", clock):
                    if usable:
                        slot, token, _ = await self.acquire()
                        self.assertEqual(token, "timed-wrapper-token")
                        self.host._token_release(acc, slot)
                    else:
                        with self.assertRaisesRegex(RuntimeError, "120-second lifetime"):
                            await self.acquire()
                self.assertEqual(acc.token_pool, {})
        self.assertEqual(device.page.evaluate.await_count, 3)

    async def test_wrapper_keeps_account_device_and_ignores_cached_parts(self):
        prepared = [await self.ready("a", "device-a"),
                    await self.ready("b", "device-b")]
        snapshots = []
        for name, (acc, device) in zip(("a", "b"), prepared):
            acc.forge_parts = {device.id: {
                "parts": {"fixture": name}, "born": self.module.time.monotonic()}}
            snapshots.append(deepcopy(acc.forge_parts))
            device.page.evaluate.return_value = "wrapper-" + name
        results = [await self.acquire("a", "device-a"),
                   await self.acquire("b", "device-b")]
        self.assertEqual([result[1:] for result in results],
                         [("wrapper-a", "device-a"), ("wrapper-b", "device-b")])
        for name, (acc, device), result, before in zip(
                ("a", "b"), prepared, results, snapshots):
            self.assertIs(self.host.accounts[name], acc)
            self.assertEqual(set(acc.dev_objs), {device.id})
            self.assertEqual(acc.forge_parts, before)
            acc.browser.new_context.assert_awaited_once()
            device.page.evaluate.assert_awaited_once_with(self.module.MINT_JS)
            self.host._token_release(acc, result[0])

    async def test_idle_maintenance_neither_mints_nor_consumes_capacity(self):
        acc, device = await self.ready()
        acc.devices[:] = [device.id]
        acc.devpool_ready, acc.project = True, "fixture-project"
        before = deepcopy(acc.lifecycle.rates.state["requests"])
        self.host._note_request = Mock(side_effect=self.module.DeviceBudgetUnavailable(1))
        self.host._run_job = AsyncMock(side_effect=AssertionError("unexpected send"))
        self.host._cool_device = Mock(side_effect=AssertionError("unexpected cooldown"))
        self.host._token_release = Mock(wraps=self.host._token_release)
        await self.cycle(acc)
        self.host._note_request.assert_not_called()
        self.host._run_job.assert_not_awaited()
        self.host._cool_device.assert_not_called()
        self.host._token_release.assert_not_called()
        self.assertEqual(acc.token_pool, {})
        self.assertEqual(acc.lifecycle.rates.state["requests"], before)
        device.page.evaluate.assert_not_awaited()

    async def test_failed_device_mint_never_uses_an_ambient_token(self):
        acc, device = await self.ready()
        self.host._new_mint_page = AsyncMock()
        device.page.evaluate.return_value = None
        for error in (None, RuntimeError("fixture wrapper failure")):
            with self.subTest(error=type(error).__name__):
                device.page.evaluate.side_effect = error
                if error is None:
                    with self.assertRaisesRegex(RuntimeError, "SDK returned no usable token"):
                        await self.acquire()
                else:
                    with self.assertRaisesRegex(RuntimeError, "fixture wrapper failure"):
                        await self.acquire()
                self.assertEqual(acc.token_pool, {})
                self.assertFalse(acc.mint_lock.locked())
        self.host._new_mint_page.assert_not_awaited()

    async def test_mismatched_token_device_is_rejected(self):
        acc, device = await self.ready()
        for other_device in (None, "device-b"):
            with self.subTest(other_device=other_device):
                self.host._mint = AsyncMock(return_value=("token", other_device))
                with self.assertRaisesRegex(RuntimeError, "browser token device mismatch"):
                    await self.acquire()
                self.assertEqual(acc.token_pool, {})
                self.host._mint.assert_awaited_once_with(
                    acc, acc.page, "a", device.id)

    async def test_failed_device_preparation_does_not_post_from_ambient_page(self):
        acc = self.account()
        context = acc.browser.next_context
        context.add_cookies.side_effect = RuntimeError("fixture preparation failure")
        self.host._pick_device_sticky = lambda _acc: "device-a"
        self.host._ensure_editor = AsyncMock()
        self.host._token_acquire = AsyncMock()
        self.host._note_request = Mock()
        self.host._run_job = AsyncMock()
        with self.assertRaisesRegex(RuntimeError, "fixture preparation failure"):
            await self.host.handle({"cmd": "ai_post", "account": "a",
                                    "path": "/fixture", "body": {}})
        self.assertTrue(context.closed)
        self.host._ensure_editor.assert_not_awaited()
        self.host._token_acquire.assert_not_awaited()
        self.host._note_request.assert_not_called()
        self.host._run_job.assert_not_awaited()

    async def test_memory_refusal_keeps_its_cause_without_minting(self):
        acc, device = await self.ready()
        failure = self.module.MemoryPressureError("Low available RAM: fixture allocation")
        self.host._dev_page = AsyncMock(side_effect=failure)
        with self.assertRaisesRegex(self.module.MemoryPressureError, "fixture allocation"):
            await self.acquire()
        device.page.evaluate.assert_not_awaited()
        self.assertEqual(acc.token_pool, {})
        self.assertFalse(acc.mint_lock.locked())
