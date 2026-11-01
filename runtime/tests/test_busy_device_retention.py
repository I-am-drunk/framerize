"""Device cleanup remains bounded while another request is still active."""
import asyncio
from copy import deepcopy
import unittest

from framerize_host.accounts.device_use import device_use
from framerize_host.accounts.device_retention import trim_idle_device_contexts
import test_device_retention as retention
from test_page_acquisition import Call


class BusyDeviceRetentionTests(unittest.IsolatedAsyncioTestCase):
    setUp = retention.DeviceRetentionTests.setUp
    account = retention.DeviceRetentionTests.account
    start = retention.DeviceRetentionTests.start
    finish = retention.DeviceRetentionTests.finish
    block = retention.DeviceRetentionTests.block
    assert_waiting = retention.DeviceRetentionTests.assert_waiting
    assert_idle = retention.DeviceRetentionTests.assert_idle
    prepare = retention.DeviceRetentionTests.prepare
    command = retention.DeviceRetentionTests.command
    populated = retention.DeviceRetentionTests.populated

    async def settle_trim(self, acc):
        if acc.trim_task is not None:
            await asyncio.wait_for(asyncio.shield(acc.trim_task), 1)

    async def test_repeated_devices_stay_bounded_during_another_request(self):
        acc = self.account()
        acc.devices[:] = ["primary"] + [f"device-{i}" for i in range(32)]
        acc.primary_dev = "primary"
        before = deepcopy(acc.lifecycle.rates.state)
        async with self.host._use_account("a"), device_use(self.host, "a", acc, "primary"):
            primary = await self.host._dev_page(acc, "a", "primary")
            for name in acc.devices[1:]:
                async with device_use(self.host, "a", acc, name):
                    await self.host._dev_page(acc, "a", name)
                await self.settle_trim(acc)
                self.assertLessEqual(sum(d.ctx is not None for d in acc.dev_objs.values()), 2)
                self.assertFalse(primary.page.is_closed())
        await self.settle_trim(acc)
        self.assertEqual(acc.lifecycle.rates.state, before)

    async def test_cancelled_request_keeps_its_device_until_raw_call_finishes(self):
        acc = await self.populated()
        device = acc.dev_objs["device-1"]
        context = device.ctx
        gate = self.block(Call())

        async def use():
            async with self.host._use_account("a"), device_use(self.host, "a", acc, device.id):
                await self.host._rpc("a", gate)

        request = self.start(use())
        await asyncio.wait_for(gate.entered.wait(), 1)
        request.cancel()
        await self.assert_waiting(request)
        self.assertEqual(device.users, 1)
        self.assertEqual(await trim_idle_device_contexts(
            self.host, "a", acc, allow_busy=True), 2)
        self.assertFalse(context.closed)
        gate.release.set()
        with self.assertRaises(asyncio.CancelledError):
            await asyncio.wait_for(request, 1)
        await self.settle_trim(acc)
        self.assertEqual(device.users, 0)
