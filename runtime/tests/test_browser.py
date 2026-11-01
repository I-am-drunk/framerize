"""Browser dependencies remain optional until the real launch boundary."""
import builtins
from contextlib import asynccontextmanager
import importlib
import os
from pathlib import Path
import sys
import tempfile
from types import ModuleType, SimpleNamespace
import unittest
from unittest.mock import AsyncMock, Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
IMPORT = builtins.__import__

def without_browser(name, *args, **kwargs):
    if name.split('.')[0] in ('camoufox', 'playwright'):
        raise ImportError('browser dependency deliberately unavailable')
    return IMPORT(name, *args, **kwargs)

class LazyBrowserRuntime(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        environment = patch.dict(os.environ, {'FRAMERIZE_STATE': temporary.name,
            'FRAMERIZE_DEVICE_POOL': '1', 'FRAMERIZE_RESERVE_PER_DAY': '0'}, clear=True)
        environment.start()
        self.addCleanup(environment.stop)
        imports = patch('builtins.__import__', side_effect=without_browser)
        imports.start()
        self.addCleanup(imports.stop)
        self.module = importlib.import_module('framerize_host.app')
        self.driver = importlib.import_module('framerize_host.browser.driver')
        settings = patch.multiple(self.module, ROOT=temporary.name,
                                  DEVICE_POOL=1, RESERVE_PER_DAY=0)
        settings.start()
        self.addCleanup(settings.stop)
        self.host = self.module.Host()

    async def test_idle_host_does_not_import_or_prepare_browser(self):
        with patch.object(self.module, 'prepared_memory_browser') as browser, \
             patch.object(self.module, 'prepared_memory_addon') as addon:
            host = self.module.Host()
            self.assertEqual(host._browser_status('fixture')['state'], 'absent')
            self.assertIsNone(host._playwright)
            self.assertIsNone(host._browser_runtime)
            browser.assert_not_called()
            addon.assert_not_called()
            await host.close()

    async def test_missing_dependency_starts_no_browser(self):
        with self.assertRaisesRegex(RuntimeError, '^browser preparation failed: RuntimeError: camoufox not installed'):
            await self.host._launch('fixture')
        self.assertIsNone(self.host._playwright)
        self.assertIsNone(self.host._browser_runtime)
        self.assertEqual(self.host._browser_status('fixture')['state'], 'absent')

    async def test_browser_launch_prepares_once_and_keeps_account_options(self):
        allocations, launches, browsers = [], [], []
        driver = SimpleNamespace(stop=AsyncMock())
        start = AsyncMock(return_value=driver)
        async def new_browser(actual_driver, **options):
            self.assertIs(actual_driver, driver)
            launches.append(options)
            browser = object()
            browsers.append(browser)
            return browser
        factories = (new_browser, Mock(return_value=SimpleNamespace(start=start)))
        @asynccontextmanager
        async def allocation(estimate):
            allocations.append(estimate)
            yield
        self.host._allocation = allocation
        async def initialize(account, profile, browser):
            self.host.lifecycles[account].phase = 'resident'
            return SimpleNamespace(initialized=True, lifecycle=self.host.lifecycles[account], devices=[])
        self.host._initialize_account = AsyncMock(side_effect=initialize)
        receipts = []
        def close_receipt(browser):
            receipt = SimpleNamespace(browser=browser, close=AsyncMock())
            receipts.append(receipt)
            return receipt
        with patch.object(self.module, 'browser_runtime', return_value=factories) as load, \
             patch.object(self.module, 'prepared_memory_browser', return_value='/fixture/browser') as memory, \
             patch.object(self.module, 'prepared_memory_addon',
                return_value={'path': '/fixture/addon', 'excluded_default': 'original'}) as addon, \
             patch.object(self.module, 'BrowserCloseReceipt', side_effect=close_receipt):
            for account in ('first', 'second'):
                self.host.lifecycles[account] = self.module.AccountLifecycle()
                value = await self.host._launch(account, proxy='http://' + account + '.invalid')
                self.assertTrue(value.initialized)
            load.assert_called_once_with()
            memory.assert_called_once_with()
            addon.assert_called_once_with()
            start.assert_awaited_once_with()
            await self.host.close()
        self.assertEqual(allocations, [self.module.BROWSER_STARTUP_RESERVE] * 2)
        self.assertEqual(len(browsers), 2)
        self.assertIsNot(browsers[0], browsers[1])
        for account, options, receipt in zip(('first', 'second'), launches, receipts):
            self.assertEqual(options['proxy'], {'server': 'http://' + account + '.invalid'})
            self.assertEqual(options['executable_path'], '/fixture/browser')
            self.assertEqual(options['addons'], ['/fixture/addon'])
            self.assertEqual(options['exclude_addons'], ['original'])
            self.assertTrue(options['headless'])
            self.assertFalse(options['humanize'])
            self.assertEqual(options['timeout'], 600_000)
            self.assertFalse(options['firefox_user_prefs']['framerize.captureResponseBodies'])
            receipt.close.assert_awaited_once_with()
            self.assertEqual(self.host.lifecycles[account].phase, 'absent')
        driver.stop.assert_awaited_once_with()

    async def test_failed_preparation_is_not_cached(self):
        with patch.object(self.module, 'browser_runtime', return_value=(None, None)), \
             patch.object(self.module, 'prepared_memory_browser', side_effect=ValueError('fixture')):
            with self.assertRaisesRegex(RuntimeError, '^browser preparation failed: ValueError: fixture$'):
                await self.host._launch('fixture')
        self.assertIsNone(self.host._browser_runtime)
        self.assertIsNone(self.host._playwright)

    async def test_loader_preserves_transport_option_and_factory_identity(self):
        camoufox, playwright = ModuleType('camoufox.async_api'), ModuleType('playwright.async_api')
        camoufox.AsyncNewBrowser, playwright.async_playwright = object(), object()
        modules = {'camoufox': ModuleType('camoufox'), 'camoufox.async_api': camoufox,
            'playwright': ModuleType('playwright'), 'playwright.async_api': playwright}
        for enabled, outcome in (('1', 'applied'), ('1', 'unsupported'), ('0', 'unused')):
            with self.subTest(enabled=enabled, outcome=outcome), \
                 patch('builtins.__import__', IMPORT), patch.dict(sys.modules, modules), \
                 patch.dict(os.environ, {'FRAMERIZE_LINEAR_PYTHON_PIPE': enabled}), \
                 patch.object(self.driver, 'install_driver_memory') as memory, \
                 patch.object(self.driver, 'install', return_value=outcome) as install, \
                 patch.object(self.driver, 'emit') as emit:
                self.assertEqual(self.driver.browser_runtime(),
                    (camoufox.AsyncNewBrowser, playwright.async_playwright))
                memory.assert_called_once_with(self.driver.ROOT)
                if enabled == '1':
                    install.assert_called_once_with()
                    emit.assert_called_once_with('memory.python_pipe', outcome=outcome,
                        level='warn' if outcome == 'unsupported' else 'info')
                else:
                    install.assert_not_called()
                    emit.assert_not_called()


if __name__ == '__main__':
    unittest.main(verbosity=2)
