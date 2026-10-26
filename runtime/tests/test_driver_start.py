"""Shared driver retry ownership with inert start and stop operations."""
import asyncio
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock

import test_browser as browser_fixture


class StartFactory:
    def __init__(self):
        self.first_entered, self.first_release = asyncio.Event(), asyncio.Event()
        self.retry_entered, self.retry_release = asyncio.Event(), asyncio.Event()
        self.failure = RuntimeError("fixture driver start failed")
        self.calls, self.attempts, self.drivers = 0, [], []

    def __call__(self):
        return SimpleNamespace(start=self.start)

    async def start(self):
        self.attempts.append(asyncio.current_task())
        self.calls += 1
        if self.calls == 1:
            self.first_entered.set()
            await self.first_release.wait()
            raise self.failure
        driver = SimpleNamespace(stop=AsyncMock())
        self.drivers.append(driver)
        self.retry_entered.set()
        await self.retry_release.wait()
        return driver


class DriverStartTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        browser_fixture.LazyBrowserRuntime.setUp(self)
        self.factory, self.tasks = StartFactory(), []

    def spawn(self, coroutine):
        task = asyncio.create_task(coroutine)
        self.tasks.append(task)
        return task

    async def wait(self, awaitable):
        return await asyncio.wait_for(awaitable, 3)

    async def asyncTearDown(self):
        self.factory.first_release.set()
        self.factory.retry_release.set()
        for task in self.tasks:
            task.cancel()
        await self.wait(asyncio.gather(*self.tasks, return_exceptions=True))
        await self.wait(asyncio.gather(*self.factory.attempts, return_exceptions=True))
        await self.wait(self.host.close())

    async def join_start(self, entered):
        entered.set()
        await self.host._start_driver(self.factory)

    async def overlapping_retry(self):
        async def retrying():
            with self.assertRaises(RuntimeError) as caught:
                await self.host._start_driver(self.factory)
            self.assertIs(caught.exception, self.factory.failure)
            await self.host._start_driver(self.factory)

        retry = self.spawn(retrying())
        await self.wait(self.factory.first_entered.wait())
        entered = asyncio.Event()

        async def stale_waiter():
            entered.set()
            with self.assertRaises(RuntimeError) as caught:
                await self.host._start_driver(self.factory)
            return caught.exception

        stale = self.spawn(stale_waiter())
        await self.wait(entered.wait())
        # Both waiters now shield the same first attempt. FIFO wakeups let
        # the first install its retry before the stale waiter's finalizer.
        self.factory.first_release.set()
        await self.wait(self.factory.retry_entered.wait())
        self.assertIs(await self.wait(stale), self.factory.failure)
        return retry

    async def assert_pending(self, task):
        with self.assertRaises(asyncio.TimeoutError):
            await asyncio.wait_for(asyncio.shield(task), 0.03)

    async def test_late_failed_waiter_does_not_duplicate_retry_driver(self):
        retry = await self.overlapping_retry()
        entered = asyncio.Event()
        other = self.spawn(self.join_start(entered))
        await self.wait(entered.wait())
        self.factory.retry_release.set()
        await self.wait(asyncio.gather(retry, other))
        self.assertEqual(self.factory.calls, 2)
        self.assertEqual(len(self.factory.drivers), 1)
        self.assertIs(self.host._playwright, self.factory.drivers[0])
        self.assertIsNone(self.host._driver_start_task)
        await self.wait(self.host.close())
        self.factory.drivers[0].stop.assert_awaited_once_with()
        self.assertIsNone(self.host._playwright)

    async def test_shutdown_joins_retry_after_stale_and_cancelled_waiters(self):
        retry = await self.overlapping_retry()
        retry.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await self.wait(retry)
        owned = self.factory.attempts[1]
        closing = self.spawn(self.host.close())
        await self.assert_pending(closing)
        self.assertFalse(owned.done())
        self.factory.drivers[0].stop.assert_not_awaited()
        closing.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await self.wait(closing)
        another = self.spawn(self.host.close())
        await self.assert_pending(another)
        self.factory.retry_release.set()
        await self.wait(another)
        self.assertTrue(owned.done())
        self.assertFalse(owned.cancelled())
        self.assertEqual(self.factory.calls, 2)
        self.factory.drivers[0].stop.assert_awaited_once_with()
        self.assertIsNone(self.host._playwright)

    async def test_failed_start_propagates_and_successful_retry_is_cached(self):
        self.factory.first_release.set()
        with self.assertRaises(RuntimeError) as caught:
            await self.wait(self.host._start_driver(self.factory))
        self.assertIs(caught.exception, self.factory.failure)
        self.assertIsNone(self.host._playwright)
        self.assertIsNone(self.host._driver_start_task)
        self.factory.retry_release.set()
        await self.wait(self.host._start_driver(self.factory))
        await self.wait(self.host._start_driver(self.factory))
        self.assertEqual(self.factory.calls, 2)
        self.assertIs(self.host._playwright, self.factory.drivers[0])
        self.assertIsNone(self.host._driver_start_task)
        await self.wait(self.host.close())
        self.factory.drivers[0].stop.assert_awaited_once_with()
        self.assertIsNone(self.host._playwright)

    async def test_cancelled_shared_waiters_keep_pending_start_owned(self):
        self.factory.first_release.set()
        with self.assertRaises(RuntimeError):
            await self.wait(self.host._start_driver(self.factory))
        first = self.spawn(self.host._start_driver(self.factory))
        await self.wait(self.factory.retry_entered.wait())
        entered = asyncio.Event()
        second = self.spawn(self.join_start(entered))
        await self.wait(entered.wait())
        for task in (first, second):
            task.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await self.wait(task)
        owned = self.factory.attempts[1]
        self.assertIs(self.host._driver_start_task, owned)
        self.assertFalse(owned.done())
        closing = self.spawn(self.host.close())
        await self.assert_pending(closing)
        self.factory.retry_release.set()
        await self.wait(closing)
        self.assertTrue(owned.done())
        self.assertFalse(owned.cancelled())
        self.assertEqual(self.factory.calls, 2)
        self.factory.drivers[0].stop.assert_awaited_once_with()
        self.assertIsNone(self.host._playwright)


if __name__ == "__main__":
    unittest.main(verbosity=2)
