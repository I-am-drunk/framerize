"""Real launch/close source with inert factories; no process or network proof."""
import asyncio
from contextlib import asynccontextmanager
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, Mock, patch

import test_lazy_browser_runtime as lazy_runtime

ACCOUNT = 'fixture-launch-route'
AMBIENT = 'http://ambient.invalid:8080'
REGISTERED = 'http://registered.invalid:8080'
LATER = 'http://later.invalid:8080'
UNCHANGED = object()


class SDKLaunchRouteTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        # Replace the mapping before the harness can copy any environment.
        self.environment = {}
        environment = patch.object(lazy_runtime.os, 'environ', self.environment)
        environment.start()
        self.addCleanup(environment.stop)
        lazy_runtime.LazyBrowserRuntime.setUp(self)
        self.addAsyncCleanup(self.host.close)

    async def assert_launch(self, ambient=None, registered=None, later=UNCHANGED):
        self.environment.pop('FRAMERIZE_PROXY', None)
        if ambient is not None:
            self.environment['FRAMERIZE_PROXY'] = ambient
        host = self.module.Host()
        life = self.module.AccountLifecycle()
        life.incarnation = 17
        host.lifecycles[ACCOUNT] = life
        if registered is not None:
            host.proxies[ACCOUNT] = registered
        expected = registered if registered is not None else ambient
        entered, release = asyncio.Event(), asyncio.Event()
        allocations, launches, captured = [], [], []
        browser = object()
        driver = SimpleNamespace(stop=AsyncMock())
        start = AsyncMock(return_value=driver)
        receipt = SimpleNamespace(browser=browser, close=AsyncMock())

        async def new_browser(actual_driver, **options):
            launches.append((actual_driver, options))
            entered.set()
            await release.wait()
            return browser

        factories = (new_browser, Mock(return_value=SimpleNamespace(start=start)))

        @asynccontextmanager
        async def allocation(estimate):
            allocations.append(estimate)
            yield

        async def initialize(account, profile, actual_browser):
            self.assertEqual(account, ACCOUNT)
            self.assertIs(actual_browser, browser)
            self.assertIs(life.browser, browser)
            self.assertIs(life._sdk_launch_browser, browser)
            self.assertIs(life.close_receipt, receipt)
            self.assertIs(type(life._sdk_launch_route), tuple)
            self.assertIs(type(life._sdk_launch_route[0]), int)
            self.assertEqual(life._sdk_launch_route, (17, expected))
            captured.append(life._sdk_launch_route)
            life.phase = 'resident'
            return SimpleNamespace(initialized=True)

        host._allocation = allocation
        host._initialize_account = AsyncMock(side_effect=initialize)
        with patch.object(self.module, 'browser_runtime', return_value=factories), \
             patch.object(self.module, 'prepared_memory_browser', return_value=None), \
             patch.object(self.module, 'prepared_memory_addon', return_value=None), \
             patch.object(self.module, 'BrowserCloseReceipt', return_value=receipt) as close_factory:
            launch = asyncio.create_task(host._launch(ACCOUNT))
            try:
                await asyncio.wait_for(entered.wait(), 5)
                self.assertFalse(launch.done())
                self.assertEqual(len(launches), 1)
                actual_driver, options = launches[0]
                self.assertIs(actual_driver, driver)
                self.assertIsNone(life._sdk_launch_route)
                self.assertIsNone(life._sdk_launch_browser)
                host._initialize_account.assert_not_awaited()
                if expected is None:
                    self.assertNotIn('proxy', options)
                else:
                    self.assertEqual(options['proxy'], {'server': expected})
                if later is not UNCHANGED:
                    self.environment.pop('FRAMERIZE_PROXY', None)
                    if later is not None:
                        self.environment['FRAMERIZE_PROXY'] = later
                    self.assertEqual(self.environment.get('FRAMERIZE_PROXY'), later)
                release.set()
                value = await asyncio.wait_for(asyncio.shield(launch), 5)
                self.assertTrue(value.initialized)
                self.assertEqual(value.resident_launch, (browser, 17, expected, False))
                self.assertEqual(len(captured), 1)
                self.assertIs(life._sdk_launch_route, captured[0])
                self.assertIs(life._sdk_launch_browser, browser)
                self.assertEqual(host.proxies.get(ACCOUNT), registered)
                host._initialize_account.assert_awaited_once()
                close_factory.assert_called_once_with(browser)
            finally:
                release.set()
                await asyncio.gather(launch, return_exceptions=True)
                await host.close()
        receipt.close.assert_awaited_once_with()
        driver.stop.assert_awaited_once_with()
        start.assert_awaited_once_with()
        self.assertEqual(allocations, [self.module.BROWSER_STARTUP_RESERVE])
        self.assertEqual(life.phase, 'absent')
        self.assertIsNone(life.browser)
        self.assertIsNone(life._sdk_launch_browser)
        self.assertIsNone(life.close_receipt)
        self.assertIs(life._sdk_launch_route, captured[0])
        self.assertEqual(life._sdk_launch_route, (17, expected))
        self.assertIsNone(life.close_task)
        self.assertFalse(life.activity.pending)
        self.assertIsNone(life.resident_uncertain)

    async def test_direct_launch_captures_none_and_retains_tuple_after_close(self):
        await self.assert_launch()

    async def test_ambient_launch_captures_exact_effective_route(self):
        await self.assert_launch(ambient=AMBIENT)

    async def test_registered_route_takes_precedence_over_ambient(self):
        await self.assert_launch(ambient=AMBIENT, registered=REGISTERED)

    async def test_environment_change_during_launch_preserves_capture(self):
        for ambient, later in ((None, LATER), (AMBIENT, LATER), (AMBIENT, None)):
            with self.subTest(ambient=ambient, later=later):
                await self.assert_launch(ambient=ambient, later=later)
