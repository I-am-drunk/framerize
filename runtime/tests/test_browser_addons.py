"""Avoid the optional filter engine without changing explicit browser options."""
from pathlib import Path
import sys
from types import ModuleType, SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_host.browser.driver import prepare_browser_options

class BrowserAddonTests(unittest.IsolatedAsyncioTestCase):
    async def prepare(self, **options):
        api, addons = ModuleType('camoufox.async_api'), ModuleType('camoufox.addons')
        api.launch_options = lambda **kwargs: kwargs
        addons.DefaultAddons = SimpleNamespace(UBO='built-in-filter-engine')
        modules = {'camoufox': ModuleType('camoufox'), 'camoufox.async_api': api,
                   'camoufox.addons': addons}
        with patch.dict(sys.modules, modules):
            return await prepare_browser_options(headless=True, **options)

    async def test_default_does_not_load_optional_filter_engine(self):
        options = await self.prepare(proxy={'server':'http://fixture.invalid'})
        self.assertEqual(options.get('exclude_addons'), ['built-in-filter-engine'])
        self.assertEqual(options['proxy'], {'server':'http://fixture.invalid'})

    async def test_explicit_prepared_addon_configuration_is_preserved(self):
        options = await self.prepare(addons=['/fixture/prepared-addon'], exclude_addons=['original'])
        self.assertEqual(options['addons'], ['/fixture/prepared-addon'])
        self.assertEqual(options['exclude_addons'], ['original'])
