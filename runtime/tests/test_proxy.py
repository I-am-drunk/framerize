"""Real host launch options with inert browsers, no network or SDK execution."""
import asyncio
from contextlib import asynccontextmanager
import os
from types import SimpleNamespace
import unittest
from unittest.mock import AsyncMock, Mock, patch

import test_browser as browser_fixture


class ProxyLaunchTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        browser_fixture.LazyBrowserRuntime.setUp(self)
        self.launches, self.receipts = [], []
        self.transport = SimpleNamespace(stop=AsyncMock())
        self.start = AsyncMock(return_value=self.transport)

        async def new_browser(driver, **options):
            self.assertIs(driver, self.transport)
            browser = SimpleNamespace(is_connected=lambda: True)
            self.assertEqual(set(options), {'from_options'})
            self.launches.append((browser, options['from_options']))
            return browser

        def receipt(browser):
            value = SimpleNamespace(browser=browser, close=AsyncMock())
            self.receipts.append(value)
            return value

        factories = (new_browser, Mock(return_value=SimpleNamespace(start=self.start)))
        for name, replacement in (
                ('browser_runtime', Mock(return_value=factories)),
                ('prepare_browser_options', AsyncMock(side_effect=lambda **options: options)),
                ('prepared_memory_browser', Mock(return_value=None)),
                ('prepared_memory_addon', Mock(return_value=None)),
                ('BrowserCloseReceipt', receipt)):
            guard = patch.object(self.module, name, replacement)
            guard.start()
            self.addCleanup(guard.stop)

        @asynccontextmanager
        async def allocation(_estimate):
            yield

        async def initialize(account, _profile, browser):
            value = self.module.Account(None, None, None, self.host.lifecycles[account])
            value.browser = browser
            self.host.accounts[account] = value
            return value

        self.host._allocation = allocation
        self.host._initialize_account = AsyncMock(side_effect=initialize)
        self.addAsyncCleanup(self.host.close)

    async def launch(self, account, proxy):
        revision = self.host.proxy_revisions.get(account, 0)
        reply = await self.host.handle(dict(cmd='register_proxy', account=account,
            expected_revision=revision, revision=revision + 1, proxy=proxy))
        self.assertEqual(reply['outcome'], 'registered')
        self.assertEqual(reply['route']['proxy'], proxy)
        await asyncio.wait_for(self.host._account(account), 5)
        self.assertEqual(self.host.proxies.get(account), proxy)
        return self.launches[-1][1]

    async def test_authenticated_account_proxy_reaches_browser_options(self):
        options = await self.launch('account-a', 'http://alice:secret@proxy.invalid:8080')
        self.assertEqual(options['proxy'], {'server': 'http://proxy.invalid:8080',
            'username': 'alice', 'password': 'secret'})

    async def test_encoded_credentials_and_ipv6_endpoint_are_preserved(self):
        proxy = 'https://user%40team:p%3A%2F%25%2B%C3%A9@[2001:db8::4]:8443'
        options = await self.launch('account-a', proxy)
        self.assertEqual(options['proxy'], {'server': 'https://[2001:db8::4]:8443',
            'username': 'user@team', 'password': 'p:/%+é'})

    async def test_short_authenticated_proxy_keeps_http_default(self):
        options = await self.launch('account-a', 'alice:secret@proxy.invalid:8080')
        self.assertEqual(options['proxy'], {'server': 'http://proxy.invalid:8080',
            'username': 'alice', 'password': 'secret'})

    async def test_optional_password_and_port_keep_existing_defaults(self):
        cases = (('http://alice@proxy.invalid', 'http://proxy.invalid'),
                 ('https://alice:@proxy.invalid', 'https://proxy.invalid'))
        for index, (proxy, server) in enumerate(cases):
            with self.subTest(proxy=proxy):
                options = await self.launch(f'account-{index}', proxy)
                self.assertEqual(options['proxy'],
                    {'server': server, 'username': 'alice', 'password': ''})

    async def test_global_proxy_authentication_keeps_the_existing_fallback(self):
        with patch.dict(os.environ, {'FRAMERIZE_PROXY': 'http://global:pass@ambient.invalid:80'}):
            options = await self.launch('account-a', None)
        self.assertEqual(options['proxy'], {'server': 'http://ambient.invalid:80',
            'username': 'global', 'password': 'pass'})

    async def test_account_routes_stay_separate_through_close_and_reopen(self):
        with patch.dict(os.environ, {'FRAMERIZE_PROXY': 'http://global:pass@ambient.invalid:80'}):
            first = await self.launch('account-a', 'http://alice:one@a.invalid:81')
            second = await self.launch('account-b', 'http://bob:two@b.invalid:82')
            self.assertEqual(first['proxy'], {'server': 'http://a.invalid:81',
                'username': 'alice', 'password': 'one'})
            self.assertEqual(second['proxy'], {'server': 'http://b.invalid:82',
                'username': 'bob', 'password': 'two'})
            self.assertTrue(await self.host._close_account('account-a'))
            self.receipts[0].close.assert_awaited_once_with()
            self.receipts[1].close.assert_not_awaited()
            reopened = await self.launch('account-a', 'http://alice:new@a.invalid:81')
            self.assertEqual(reopened['proxy'], {'server': 'http://a.invalid:81',
                'username': 'alice', 'password': 'new'})
            self.assertIs(self.host.accounts['account-b'].browser, self.launches[1][0])
            self.assertEqual(self.host.proxies['account-b'], 'http://bob:two@b.invalid:82')
            self.assertEqual(first['proxy']['password'], 'one')
            self.assertEqual(second['proxy']['password'], 'two')
            self.assertIsNot(reopened['proxy'], first['proxy'])
            self.start.assert_awaited_once_with()
            await self.host.close()
        for receipt in self.receipts:
            receipt.close.assert_awaited_once_with()
        for life in self.host.lifecycles.values():
            self.assertEqual(life.phase, 'absent')
            self.assertFalse(life.activity.pending)
        self.transport.stop.assert_awaited_once_with()

    async def test_unauthenticated_proxy_formats_are_unchanged(self):
        proxies = ('http://proxy.invalid', 'https://proxy.invalid:8443',
                   'proxy.invalid:8080', 'http://[2001:db8::4]:8080',
                   'socks4://proxy.invalid:1080', 'socks5://proxy.invalid:1080')
        for index, proxy in enumerate(proxies):
            with self.subTest(proxy=proxy):
                options = await self.launch(f'account-{index}', proxy)
                self.assertEqual(options['proxy'], {'server': proxy})

    async def test_direct_launch_has_no_proxy_options(self):
        options = await self.launch('account-a', None)
        self.assertNotIn('proxy', options)
