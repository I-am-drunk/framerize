"""Idle maintenance never sends synthetic AI requests or interaction signals."""
import asyncio
from copy import deepcopy
import unittest
from unittest.mock import AsyncMock, Mock, patch

import test_device_context_creation as creation
import test_page_acquisition as acquisition


class HeartbeatRetentionTests(unittest.IsolatedAsyncioTestCase):
    setUp = creation.DeviceContextCreationTests.setUp
    finish = creation.DeviceContextCreationTests.finish
    account = creation.DeviceContextCreationTests.account
    prepare = creation.DeviceContextCreationTests.prepare
    command = creation.DeviceContextCreationTests.command
    start = acquisition.PageAcquisitionTests.start
    block = acquisition.PageAcquisitionTests.block

    async def cycle(self, acc):
        with patch.object(self.module, "memory_snapshot", return_value={
                "allocation_headroom_bytes": 4 * 1024**3}):
            await self.host._monitor_once()

    def ready(self, acc):
        acc.devices[:] = [f"device-{index}" for index in range(64)]
        acc.devpool_ready, acc.project = True, "fixture-project"
        self.host._dev_page = AsyncMock(side_effect=AssertionError("must not create"))
        self.host._token_acquire = AsyncMock(return_value=("token-id", "token", "device-1"))
        self.host._run_job = AsyncMock(return_value={"status": 200})
        self.host._token_release = Mock()

    async def test_dormant_history_does_not_open_or_charge_any_device(self):
        acc = self.account()
        self.ready(acc)
        before = deepcopy(acc.lifecycle.rates.state)
        await self.cycle(acc)
        self.host._dev_page.assert_not_awaited()
        self.host._token_acquire.assert_not_awaited()
        self.host._run_job.assert_not_awaited()
        acc.browser.new_context.assert_not_awaited()
        self.assertEqual(acc.lifecycle.rates.state, before)

    async def test_existing_warm_page_sends_no_background_requests(self):
        acc = self.account()
        device = await self.prepare(device="device-1")
        self.ready(acc)
        await self.cycle(acc)
        self.host._dev_page.assert_not_awaited()
        self.host._token_acquire.assert_not_awaited()
        self.host._run_job.assert_not_awaited()
        self.host._token_release.assert_not_called()
        acc.browser.new_context.assert_awaited_once()
        self.assertEqual(list(acc.dev_objs), ["device-1"])

    async def test_closed_cold_and_locked_pages_stay_untouched(self):
        acc = self.account()
        device = await self.prepare(device="device-1")
        self.ready(acc)
        before = deepcopy(acc.lifecycle.rates.state)
        for state in ("closed", "cold", "locked", "cooling"):
            with self.subTest(state=state):
                device.page.closed = state == "closed"
                device.warm = state != "cold"
                if state == "locked":
                    await device.lock.acquire()
                if state == "cooling":
                    acc.dev_cooling[device.id] = self.module.time.monotonic() + 600
                try:
                    await self.cycle(acc)
                finally:
                    if device.lock.locked():
                        device.lock.release()
                    acc.dev_cooling.clear()
        self.host._dev_page.assert_not_awaited()
        self.host._token_acquire.assert_not_awaited()
        self.host._run_job.assert_not_awaited()
        self.assertEqual(acc.lifecycle.rates.state, before)
        acc.browser.new_context.assert_awaited_once()
