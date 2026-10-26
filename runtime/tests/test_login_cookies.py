"""Login cookie selection through inert account-owned browser ports."""
import asyncio
import json
import unittest
from unittest.mock import AsyncMock, patch
from urllib.parse import urlsplit

import test_browser as browser_fixture
from test_lifecycle import FakeBrowser


def cookie(domain, value, path="/"):
    return dict(name="session", value=value, domain=domain, path=path)


class LoginPage:
    def __init__(self, evaluate):
        self.evaluate = evaluate

    def is_closed(self):
        return False


class LoginContext:
    def __init__(self, cookies):
        self.saved, self.calls = cookies, []
        self.page = LoginPage(self.evaluate)

    def applicable(self, urls):
        if isinstance(urls, str):
            urls = [urls]
        urls = [urlsplit(url) for url in urls]
        return [item for item in self.saved if any(
            (url.hostname == item["domain"] or (item["domain"].startswith(".")
                and (url.hostname == item["domain"][1:]
                     or url.hostname.endswith(item["domain"]))))
            and (url.path or "/").startswith(item["path"]) for url in urls)]

    async def cookies(self, urls):
        self.calls.append("cookies")
        return self.applicable(urls)

    async def evaluate(self, expression):
        if "/auth/v4/continue" in expression:
            self.calls.append("continue")
            return dict(status=200, body="{}")
        assert "/web/users/me" in expression, "unexpected browser evaluation"
        self.calls.append("me")
        session = next((item["value"] for item in self.applicable("https://api.framer.com")
                        if item["name"] == "session"), None)
        if session is None:
            return dict(status=401, body="fixture signed out")
        return dict(status=200, body=json.dumps({"id": "user-" + session}))


class LoginCookieTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        browser_fixture.LazyBrowserRuntime.setUp(self)
        self.host._launch = AsyncMock(side_effect=AssertionError("unexpected launch"))
        for target, name in ((self.module, "emit"), (self.module, "trace"),
                             (self.driver, "verify_close_protocol")):
            guard = patch.object(target, name, lambda *args, **kwargs: None)
            guard.start()
            self.addCleanup(guard.stop)
        self.addAsyncCleanup(self.finish)

    async def finish(self):
        await asyncio.wait_for(self.host.close(), 2)
        for life in self.host.lifecycles.values():
            self.assertFalse(life.activity.pending)
            self.assertFalse(life.activity.activities)
            self.assertEqual(life.phase, "absent")

    def account(self, name, cookies):
        life = self.module.AccountLifecycle()
        ctx, browser = LoginContext(cookies), FakeBrowser()
        life.browser, life.phase = browser, "resident"
        life.close_receipt = self.module.BrowserCloseReceipt(browser)
        acc = self.module.Account(None, ctx, ctx.page, life)
        acc.browser = browser
        self.host.accounts[name], self.host.lifecycles[name] = acc, life
        return ctx

    async def complete(self, account):
        result = await asyncio.wait_for(self.host.handle(dict(
            cmd="complete", account=account, token="fixture-email-link")), 2)
        acc = self.host.accounts[account]
        self.assertEqual(acc.ctx.calls, ["continue", "me", "cookies"])
        self.assertFalse(acc.borrowed)
        self.assertFalse(acc.lifecycle.activity.pending)
        self.assertFalse(acc.lifecycle.activity.activities)
        return result

    async def test_website_session_does_not_replace_each_accounts_api_cookie(self):
        for name, domain in (("a", ".framer.com"), ("b", "api.framer.com")):
            self.account(name, [cookie("framer.com", "website-" + name),
                cookie("framer.com", "login-" + name, "/login"),
                cookie(domain, "api-" + name)])
        for name in ("a", "b"):
            result = await self.complete(name)
            self.assertEqual(result, {"cookie_header": "session=api-" + name,
                                      "me": {"id": "user-api-" + name}})

    async def test_host_only_api_session_is_exported(self):
        self.account("a", [cookie("api.framer.com", "api-only")])
        result = await self.complete("a")
        self.assertEqual(result, {"cookie_header": "session=api-only",
                                  "me": {"id": "user-api-only"}})

    async def test_shared_framer_session_remains_supported(self):
        self.account("a", [cookie(".framer.com", "shared")])
        result = await self.complete("a")
        self.assertEqual(result, {"cookie_header": "session=shared",
                                  "me": {"id": "user-shared"}})

    async def test_failed_identity_lookup_does_not_export_website_session(self):
        ctx = self.account("a", [cookie("framer.com", "website-only")])
        with self.assertRaisesRegex(RuntimeError, "^me 401: fixture signed out$"):
            await asyncio.wait_for(self.host.handle(dict(cmd="complete", account="a",
                token="fixture-email-link")), 2)
        self.assertEqual(ctx.calls, ["continue", "me"])
        self.assertFalse(self.host.accounts["a"].borrowed)


if __name__ == "__main__":
    unittest.main()
