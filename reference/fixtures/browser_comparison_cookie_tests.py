"""Cookie-scope regression cases; no browser, SDK or network."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('cookie_comparison_fixture',
    Path(__file__).with_name('browser_comparison_tests.py'))
F = importlib.util.module_from_spec(spec)
spec.loader.exec_module(F)
C = F.C
URL = 'https://api.framer.com/ai/v3/responses/'

def cookie(domain='.framer.com', **changes):
    return dict(name='__cuid', value=F.DEVICE, domain=domain,
                path='/', secure=True) | changes

class CookieSelectionTests(unittest.TestCase):
    def test_host_only_parent_is_excluded_from_api_cookie_jar(self):
        host, parent = cookie('framer.com'), cookie()
        self.assertEqual(C.cookies_for_url([host, parent], URL), [parent])

    def test_exact_host_only_and_parent_domain_cookies_are_preserved(self):
        values = [cookie('api.framer.com'), cookie()]
        self.assertEqual(C.cookies_for_url(values, URL), values)

    def test_domain_requires_a_label_boundary(self):
        values = [cookie('.evilframer.com'), cookie('framer.com.evil'), cookie('.api.framer.com.evil')]
        self.assertEqual(C.cookies_for_url(values, URL), [])
        self.assertEqual(C.cookies_for_url([cookie()], 'https://evilframer.com/'), [])

    def test_path_requires_a_boundary_and_secure_requires_https(self):
        yes, no = cookie(path='/ai'), cookie(path='/a')
        self.assertEqual(C.cookies_for_url([yes, no], URL), [yes])
        plain = cookie(secure=False)
        self.assertEqual(C.cookies_for_url([cookie(), plain], URL.replace('https:', 'http:')), [plain])

    def test_malformed_scope_metadata_is_refused(self):
        for changes in ({'domain':None}, {'path':'relative'}, {'secure':1}):
            with self.subTest(changes=changes), self.assertRaises(ValueError):
                C.cookies_for_url([cookie(**changes)], URL)
        for rows in (None, [cookie()] * 257, [None]):
            with self.assertRaises(ValueError):
                C.cookies_for_url(rows, URL)

class CookieComparisonTests(unittest.IsolatedAsyncioTestCase):
    asyncSetUp = F.BrowserComparisonTests.asyncSetUp
    asyncTearDown = F.BrowserComparisonTests.asyncTearDown
    fixture = F.BrowserComparisonTests.fixture

    async def test_sdk_host_and_domain_pair_keeps_one_api_cookie(self):
        value = self.fixture()
        value.context.rows = [dict(name='session',value=F.OLD.LOGIN),
                              cookie('framer.com'), cookie()]
        context = await value.prepare_context()
        self.assertEqual(context['cookie_header'].count('__cuid='), 1)
        self.assertTrue((await value.baseline())['accepted'])
        await value.native()
        self.assertEqual((value.mints,value.fetches), (2,1))

    async def test_conflicting_matching_domain_cookies_still_refuse_before_mint(self):
        value = self.fixture()
        value.context.rows += [cookie(path='/ai',value='conflicting-fixture-device')]
        with self.assertRaises(RuntimeError):
            await value.prepare_context()
        self.assertEqual((value.mints,value.fetches), (0,0))

    async def test_host_only_login_is_not_forwarded_to_api(self):
        value = self.fixture()
        value.context.rows.append(cookie('framer.com',name='session',value='wrong-host-login'))
        context = await value.prepare_context()
        self.assertEqual(context['cookie_header'].count('session='), 1)
        self.assertNotIn('wrong-host-login', context['cookie_header'])

if __name__ == '__main__':
    unittest.main()
