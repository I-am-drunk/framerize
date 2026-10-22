"""Offline contextual authorization cases using explicit page and RPC fakes."""
import ast
import asyncio
from contextlib import asynccontextmanager
import importlib.util
from pathlib import Path
import sys
import traceback
from types import SimpleNamespace
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parents[2] / "scripts"
SPEC = importlib.util.spec_from_file_location(
    "_fixture_contextual_authorization", SCRIPTS / "contextual_authorization.py"
)
CONTEXT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CONTEXT)
ACCOUNT = "fixture-account"
PROJECT = "AB12cd34EF56gh78IJ90"
PAGE_URL = "https://framer.com/projects/" + PROJECT
SDK_URL = "https://framer.com/fixture-wrapper.js"
LOGIN = "fixture-login"
UNSET = object()


def request(path="/ai/v3/responses/", **changes):
    value = {"project": PROJECT, "path": path, "cookie_header": "session=" + LOGIN}
    value.update(changes)
    return value


class FakeScope:
    def __init__(self):
        self.contextual_authorization = True
        self.failure = None
        self.stopped = asyncio.Event()
        self.pending, self.submitted = set(), []
        self.idle = asyncio.Event()
        self.idle.set()

    def rpc(self, coroutine):
        task = asyncio.create_task(coroutine)
        self.pending.add(task)
        self.submitted.append(task)
        self.idle.clear()
        task.add_done_callback(self._finished)
        return task

    def _finished(self, task):
        self.pending.discard(task)
        if not task.cancelled():
            task.exception()
        if not self.pending:
            self.idle.set()

    async def drain(self):
        if self.pending:
            await asyncio.gather(*tuple(self.pending), return_exceptions=True)
        await self.idle.wait()


class FakePage:
    def __init__(self, calls):
        self.calls = calls
        self.url = "about:blank"
        self.goto_result = self.changed_after_sdk = None
        self.result = UNSET
        self.sdk_started, self.sdk_release = asyncio.Event(), asyncio.Event()
        self.sdk_release.set()
        self.fail_sdk = False

    async def goto(self, url, *, wait_until, timeout):
        self.calls.append(("goto", url, wait_until, timeout))
        self.url = self.goto_result if self.goto_result is not None else url

    async def evaluate(self, script, sdk_url):
        self.calls.append(("evaluate", script, sdk_url))
        self.sdk_started.set()
        await self.sdk_release.wait()
        if self.fail_sdk:
            raise RuntimeError("fixture-private-error")
        value = self.result if self.result is not UNSET else {
            "before": self.url, "after": self.url, "token": "fixture-sdk-material"
        }
        if self.changed_after_sdk is not None:
            self.url = self.changed_after_sdk
        return value


class FakeContext:
    def __init__(self, page, calls):
        self.page, self.calls = page, calls
        self.rows = [{"name": "session", "value": LOGIN},
                     {"name": "context", "value": "current-value"}]
        self.changed_during_cookies = None
        self.on_cookies = None
        self.page_started, self.page_release = asyncio.Event(), asyncio.Event()
        self.page_release.set()

    async def new_page(self):
        self.calls.append(("new_page",))
        self.page_started.set()
        await self.page_release.wait()
        return self.page

    async def add_cookies(self, rows):
        self.calls.append(("add_cookies", rows))

    async def cookies(self, url):
        self.calls.append(("cookies", url))
        if self.on_cookies is not None:
            self.on_cookies()
        if self.changed_during_cookies is not None:
            self.page.url = self.changed_during_cookies
        return self.rows


class FakeBrowser:
    def __init__(self, context, calls):
        self.context, self.calls = context, calls

    async def new_context(self):
        self.calls.append(("new_context",))
        return self.context


class FakeAccount:
    def __init__(self, unused, context, page, lifecycle=None):
        self.ctx, self.page = context, page
        self.lifecycle = lifecycle


class Harness:
    def __init__(self):
        self.calls, self.scope = [], FakeScope()
        self.account_id = ACCOUNT
        self.page = FakePage(self.calls)
        self.context = FakeContext(self.page, self.calls)
        self.browser = FakeBrowser(self.context, self.calls)
        self.account = FakeAccount(None, self.context, self.page)
        self.account._observed_scope = self.scope
        self.host = SimpleNamespace(
            accounts={ACCOUNT: self.account},
            lifecycles={ACCOUNT: SimpleNamespace()},
            _observed_scope=lambda account: self.scope,
            _observed=SimpleNamespace(accounts={ACCOUNT: self.scope}),
        )

    async def prepare(self, path="/ai/v3/responses/", **changes):
        plan = CONTEXT.validate_request(request(path, **changes))
        return await CONTEXT.prepare(self.host, self.account_id, SDK_URL, plan)


def host_method(name, namespace):
    tree = ast.parse((SCRIPTS / "camoufox_host.py").read_text())
    node = next(n for n in ast.walk(tree) if isinstance(n, ast.AsyncFunctionDef)
                and n.name == name)
    module = ast.Module(body=[node], type_ignores=[])
    exec(compile(module, "<isolated host method>", "exec"), namespace)
    return namespace[name]


def host_initializer():
    return host_method("_initialize_account", {"Account": FakeAccount})


async def launch_fixture(observed, explicit=None, saved=None):
    calls, launches, browser = [], [], object()

    async def new_browser(driver, **kwargs):
        launches.append(kwargs)
        return browser

    async def initialize(account, profile, value):
        calls.append(("initialize", account, profile, value))
        return value

    @asynccontextmanager
    async def allocation(reserve):
        calls.append(("allocation", reserve))
        yield

    scope = SimpleNamespace(
        begin_launch=lambda: calls.append(("begin_launch",)),
        browser_started=lambda value: calls.append(("browser_started", value)),
        fail=lambda reason: calls.append(("fail", reason)),
    )
    host = SimpleNamespace(
        proxies={} if saved is None else {ACCOUNT: saved}, _memory_addon=None,
        _memory_browser=None, _playwright=object(), _allocation=allocation,
        _initialize_account=initialize,
        lifecycles={ACCOUNT: SimpleNamespace(incarnation=1)},
        _observed_scope=lambda account: scope if observed else None,
        _observed=SimpleNamespace(accounts={ACCOUNT: scope}) if observed else None,
    )
    fake_os = SimpleNamespace(
        environ={"FRAMERIZE_PROXY": "http://ambient.invalid:9100"},
        path=SimpleNamespace(join=lambda base, account: base + "/" + account),
        makedirs=lambda path, **kwargs: calls.append(("mkdir", path)),
    )
    launch = host_method("_launch", dict(os=fake_os, ROOT="fixture-root",
        time=SimpleNamespace(monotonic=lambda: 0), BROWSER_STARTUP_RESERVE=1,
        trace=lambda *args: None, emit=lambda *args, **kwargs: None,
        AsyncNewBrowser=new_browser, BrowserCloseReceipt=lambda value: object()))
    result = await launch(host, ACCOUNT, explicit)
    return host, result, launches, calls


class ContextualAuthorizationTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.fixtures, self.waiters = [], []

    def fixture(self):
        value = Harness()
        self.fixtures.append(value)
        return value

    def start(self, fixture, operation=None):
        task = asyncio.create_task(fixture.prepare() if operation is None else operation)
        self.waiters.append(task)
        return task

    async def rejected(self, fixture):
        with self.assertRaisesRegex(RuntimeError, "^contextual authorization failed$") as caught:
            await fixture.prepare()
        return caught.exception

    def call_names(self, fixture):
        return [call[0] for call in fixture.calls]

    async def wait_for(self, awaitable):
        return await asyncio.wait_for(awaitable, 2)

    async def asyncTearDown(self):
        for value in self.fixtures:
            value.page.sdk_release.set()
            value.context.page_release.set()
        for task in self.waiters:
            if not task.done():
                task.cancel()
        if self.waiters:
            await asyncio.gather(*self.waiters, return_exceptions=True)
        await asyncio.wait_for(asyncio.gather(
            *(value.scope.drain() for value in self.fixtures)
        ), 2)

    async def test_host_initializer_uses_owned_context_without_legacy_warmup(self):
        value = self.fixture()
        value.host.accounts.clear()
        with patch.dict(sys.modules, {"contextual_authorization": CONTEXT}):
            account = await host_initializer()(
                value.host, ACCOUNT, "unused-fixture-profile", value.browser
            )
        self.assertEqual(self.call_names(value), ["new_context", "new_page"])
        self.assertEqual(value.page.url, "about:blank")
        self.assertIs(account, value.host.accounts[ACCOUNT])
        self.assertIs(account._observed_scope, value.scope)
        self.assertIs(account.browser, value.browser)
        self.assertIs(account.ctx, value.context)
        self.assertIs(account.lifecycle, value.host.lifecycles[ACCOUNT])
        self.assertIs(account.page, value.page)
        self.assertEqual(len(value.scope.submitted), 2)
        await self.wait_for(value.scope.drain())

    async def test_owner_mismatch_rejects_before_touching_page_or_cookies(self):
        for mismatch in ("disabled", "scope", "missing-account", "missing-scope"):
            with self.subTest(mismatch=mismatch):
                value = self.fixture()
                if mismatch == "disabled":
                    value.scope.contextual_authorization = False
                elif mismatch == "scope":
                    value.account._observed_scope = object()
                elif mismatch == "missing-account":
                    value.host.accounts.clear()
                else:
                    value.host._observed.accounts.clear()
                await self.rejected(value)
                self.assertEqual(value.calls, [])
                self.assertEqual(value.scope.submitted, [])

    async def test_initializer_cancellation_keeps_page_rpc_owned(self):
        value = self.fixture()
        value.host.accounts.clear()
        value.context.page_release.clear()
        with patch.dict(sys.modules, {"contextual_authorization": CONTEXT}):
            waiter = self.start(value, host_initializer()(
                value.host, ACCOUNT, "unused-fixture-profile", value.browser
            ))
            await self.wait_for(value.context.page_started.wait())
            owned_rpc = value.scope.submitted[-1]
            waiter.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await waiter
        self.assertIn(owned_rpc, value.scope.pending)
        self.assertFalse(owned_rpc.done())
        self.assertEqual(value.host.accounts, {})
        value.context.page_release.set()
        await self.wait_for(value.scope.drain())
        self.assertIs(owned_rpc.result(), value.page)
        self.assertEqual(value.host.accounts, {})
        self.assertEqual(self.call_names(value), ["new_context", "new_page"])

    def test_request_accepts_only_exact_routes_and_bounded_project_ids(self):
        for path in ("/ai/v3/responses/", "/ai/v3/messages/"):
            for project in (PROJECT, "a", "fixture-project_1", "A" * 256):
                with self.subTest(path=path, project_length=len(project)):
                    self.assertEqual(
                        CONTEXT.validate_request(request(path, project=project)),
                        (project, path, LOGIN),
                    )

    def test_request_rejects_ambiguous_project_or_route_before_browser_use(self):
        projects = (None, True, 1, [], "", "A" * 257, "bad/project",
                    "bad space", "bad?query", "bad#fragment", "é")
        for project in projects:
            with self.subTest(project=project), self.assertRaises(ValueError):
                CONTEXT.validate_request(request(project=project))
        paths = (None, [], "/ai/v3/responses", "/ai/v3/openai/responses/",
                 "/ai/v3/responses/?extra=1", "/auth/ai/access-token",
                 "https://api.framer.com/ai/v3/responses/")
        for path in paths:
            with self.subTest(path=path), self.assertRaises(ValueError):
                CONTEXT.validate_request(request(path))

    def test_private_login_rejects_missing_duplicate_or_injected_material(self):
        invalid = (None, [], "", "theme=dark", "session=", "Session=" + LOGIN,
                   "session=" + LOGIN + "; session=" + LOGIN,
                   "session=" + LOGIN + "; session=other",
                   "session=" + LOGIN + "; orphan", "=value; session=" + LOGIN)
        for header in invalid:
            with self.subTest(header=header), self.assertRaises(ValueError):
                CONTEXT.validate_request(request(cookie_header=header))
        for suffix in ("\r\nInjected: 1", "\n", "\t", "\0", "\x7f", "é",
                       "a b", "a,b", '"quoted"', "a\\b"):
            with self.subTest(suffix=suffix), self.assertRaises(ValueError):
                CONTEXT.validate_request(request(
                    cookie_header="session=" + LOGIN + "; context=" + suffix
                ))

    def test_cookie_header_preserves_opaque_values_and_inclusive_byte_limit(self):
        header = "empty=; session=fixture=login==; context=fresh%2Bvalue"
        self.assertEqual(CONTEXT.login_cookie(header), "fixture=login==")
        prefix = "session=" + LOGIN + "; context="
        maximum = prefix + "x" * (16 * 1024 - len(prefix))
        self.assertEqual(len(maximum.encode()), 16 * 1024)
        self.assertEqual(CONTEXT.login_cookie(maximum), LOGIN)
        with self.assertRaises(ValueError):
            CONTEXT.login_cookie(maximum + "x")

    def test_snapshot_is_bounded_by_row_count_and_total_header_size(self):
        login = {"name": "session", "value": LOGIN}
        rows = [login] + [{"name": "k" + str(i), "value": ""} for i in range(255)]
        self.assertEqual(CONTEXT.login_cookie(CONTEXT.snapshot_header(rows)), LOGIN)
        with self.assertRaises(ValueError):
            CONTEXT.snapshot_header(rows + [{"name": "extra", "value": ""}])
        prefix = "session=" + LOGIN + "; context="
        rows = [login, {"name": "context", "value": "x" * (16 * 1024 - len(prefix))}]
        maximum = CONTEXT.snapshot_header(rows)
        self.assertEqual(len(maximum.encode()), 16 * 1024)
        self.assertEqual(maximum, prefix + rows[1]["value"])
        rows[1]["value"] += "x"
        with self.assertRaises(ValueError):
            CONTEXT.snapshot_header(rows)

    def test_snapshot_rejects_malformed_rows_and_cookie_delimiter_injection(self):
        login = {"name": "session", "value": LOGIN}
        for rows in (None, {}, (), [], [None], [login, login]):
            with self.subTest(rows=rows), self.assertRaises(ValueError):
                CONTEXT.snapshot_header(rows)
        invalid = ({}, {"name": "context"}, {"name": None, "value": "x"},
                   {"name": "context", "value": None})
        invalid += tuple({"name": name, "value": "x"} for name in
                         ("", "bad name", "bad:name", "é", "bad;name"))
        invalid += tuple({"name": "context", "value": value} for value in
                         ("a;b", "a,b", '"quoted"', "a\\b", "a b", "a\nb", "é"))
        for row in invalid:
            with self.subTest(row=row), self.assertRaises(ValueError):
                CONTEXT.snapshot_header([login, row])

    async def test_prepare_binds_exact_route_page_wrapper_and_current_cookie_jar(self):
        for path in ("/ai/v3/responses/", "/ai/v3/messages/"):
            with self.subTest(path=path):
                value = self.fixture()
                result = await value.prepare(
                    path, cookie_header="session=" + LOGIN + "; context=stale"
                )
                self.assertEqual(result, {
                    "project": PROJECT, "path": path, "page_url": PAGE_URL,
                    "source": "framer_wrapper_direct", "token": "fixture-sdk-material",
                    "cookie_header": "session=" + LOGIN + "; context=current-value",
                })
                self.assertEqual(self.call_names(value),
                                 ["add_cookies", "goto", "evaluate", "cookies"])
                self.assertEqual(value.calls[0][1], [{
                    "name": "session", "value": LOGIN, "domain": ".framer.com",
                    "path": "/", "secure": True, "httpOnly": True,
                }])
                self.assertEqual(value.calls[1], ("goto", PAGE_URL, "domcontentloaded", 45000))
                self.assertEqual(value.calls[2], ("evaluate", CONTEXT.DIRECT_SDK, SDK_URL))
                self.assertEqual(value.calls[3], ("cookies", "https://api.framer.com" + path))
                self.assertEqual(len(value.scope.submitted), 4)
                await self.wait_for(value.scope.drain())

    async def test_same_project_slug_and_view_query_keep_exact_returned_page(self):
        value = self.fixture()
        value.page.goto_result = "https://framer.com/projects/title--" + PROJECT + "/?node=1#view"
        result = await value.prepare()
        self.assertEqual(result["page_url"], value.page.goto_result)
        self.assertEqual(result["project"], PROJECT)

    async def test_wrong_initial_page_is_rejected_before_sdk_or_cookie_read(self):
        wrong = ("https://framer.com/login", PAGE_URL + "other", PAGE_URL + "/child",
                 "https://framer.com/projects/prefix" + PROJECT,
                 "https://framer.com/projects/wrong?project=" + PROJECT,
                 PAGE_URL.replace("https:", "http:"),
                 PAGE_URL.replace("framer.com", "framer.com.example.test"),
                 PAGE_URL.replace("framer.com", "framer.com:8443"),
                 PAGE_URL.replace("framer.com", "fixture@framer.com"))
        for page in wrong:
            with self.subTest(page=page):
                value = self.fixture()
                value.page.goto_result = page
                await self.rejected(value)
                self.assertEqual(self.call_names(value), ["add_cookies", "goto"])

    async def test_sdk_result_requires_matching_before_and_after_project(self):
        good = {"before": PAGE_URL, "after": PAGE_URL, "token": "fixture-sdk-material"}
        invalid = (None, [], True, "unexpected", {},
                   {key: val for key, val in good.items() if key != "before"},
                   dict(good, before="https://framer.com/login"),
                   dict(good, after="https://framer.com/projects/other"))
        for result in invalid:
            with self.subTest(result=result):
                value = self.fixture()
                value.page.result = result
                await self.rejected(value)
                self.assertEqual(self.call_names(value), ["add_cookies", "goto", "evaluate"])

    async def test_sdk_token_requires_nonempty_string_with_bounded_utf8_bytes(self):
        for token in (None, True, 1, [], b"fixture", "", "x" * 16385, "é" * 8193):
            with self.subTest(token_type=type(token).__name__):
                value = self.fixture()
                value.page.result = {"before": PAGE_URL, "after": PAGE_URL, "token": token}
                await self.rejected(value)
                self.assertNotIn("cookies", self.call_names(value))
        value = self.fixture()
        value.page.result = {"before": PAGE_URL, "after": PAGE_URL, "token": "x" * 16384}
        result = await value.prepare()
        self.assertEqual(result["token"], "x" * 16384)

    async def test_sdk_failure_has_only_redacted_public_error(self):
        value = self.fixture()
        value.page.fail_sdk = True
        error = await self.rejected(value)
        self.assertIsNone(error.__cause__)
        self.assertTrue(error.__suppress_context__)
        self.assertNotIn("fixture-private-error", "".join(traceback.format_exception(error)))
        self.assertNotIn("cookies", self.call_names(value))

    async def test_changed_missing_or_ambiguous_current_login_is_rejected(self):
        login = {"name": "session", "value": LOGIN}
        rows = ([{"name": "session", "value": "another-login"}],
                [{"name": "session", "value": LOGIN + "extra"}],
                [{"name": "session", "value": ""}],
                [{"name": "context", "value": "current"}], [login, login])
        for snapshot in rows:
            with self.subTest(snapshot=snapshot):
                value = self.fixture()
                value.context.rows = snapshot
                await self.rejected(value)
                self.assertEqual(self.call_names(value),
                                 ["add_cookies", "goto", "evaluate", "cookies"])

    async def test_navigation_change_after_sdk_or_during_cookie_read_rejects(self):
        for moment in ("after-sdk", "during-cookies"):
            with self.subTest(moment=moment):
                value = self.fixture()
                if moment == "after-sdk":
                    value.page.changed_after_sdk = "https://framer.com/projects/other"
                else:
                    value.context.changed_during_cookies = "https://example.test/"
                await self.rejected(value)
                self.assertEqual(value.calls[-1][0], "cookies")

    async def test_stopped_or_failed_scope_never_starts_a_new_rpc(self):
        for failed in (False, True):
            value = self.fixture()
            if failed:
                value.scope.failure = "fixture failure"
            else:
                value.scope.stopped.set()
            await self.rejected(value)
            self.assertEqual(value.calls, [])
            self.assertEqual(value.scope.submitted, [])

    async def test_stop_during_sdk_keeps_rpc_owned_but_prevents_next_operation(self):
        value = self.fixture()
        value.page.sdk_release.clear()
        waiter = self.start(value)
        await self.wait_for(value.page.sdk_started.wait())
        owned_rpc = value.scope.submitted[-1]
        value.scope.stopped.set()
        self.assertIn(owned_rpc, value.scope.pending)
        self.assertFalse(owned_rpc.done())
        value.page.sdk_release.set()
        with self.assertRaisesRegex(RuntimeError, "^contextual authorization failed$"):
            await self.wait_for(waiter)
        await self.wait_for(value.scope.drain())
        self.assertFalse(owned_rpc.cancelled())
        self.assertNotIn("cookies", self.call_names(value))

    async def test_stop_during_cookie_read_prevents_credential_return(self):
        value = self.fixture()
        value.context.on_cookies = value.scope.stopped.set
        await self.rejected(value)
        self.assertEqual(self.call_names(value), ["add_cookies", "goto", "evaluate", "cookies"])
        await self.wait_for(value.scope.drain())
        self.assertTrue(value.scope.stopped.is_set())

    async def test_cancelled_waiter_retains_sdk_rpc_until_success_or_failure(self):
        for fails in (False, True):
            with self.subTest(rpc_fails=fails):
                value = self.fixture()
                value.page.fail_sdk = fails
                value.page.sdk_release.clear()
                waiter = self.start(value)
                await self.wait_for(value.page.sdk_started.wait())
                owned_rpc = value.scope.submitted[-1]
                waiter.cancel()
                with self.assertRaises(asyncio.CancelledError):
                    await waiter
                self.assertIn(owned_rpc, value.scope.pending)
                self.assertFalse(owned_rpc.done())
                value.page.sdk_release.set()
                await self.wait_for(value.scope.drain())
                self.assertFalse(owned_rpc.cancelled())
                if fails:
                    self.assertIsInstance(owned_rpc.exception(), RuntimeError)
                else:
                    self.assertEqual(owned_rpc.result()["token"], "fixture-sdk-material")
                self.assertNotIn("cookies", self.call_names(value))
                self.assertEqual(value.scope.pending, set())

    async def test_shared_host_selects_only_the_requested_account_context(self):
        first, second = self.fixture(), self.fixture()
        second.account_id = "fixture-second-account"
        first.host.accounts[second.account_id] = second.account
        first.host._observed.accounts[second.account_id] = second.scope
        second.host = first.host
        second.context.rows[0]["value"] = "second-login"
        second.context.rows[1]["value"] = "second-current-value"
        left, right = await asyncio.gather(
            first.prepare(), second.prepare(cookie_header="session=second-login")
        )
        self.assertEqual(first.calls[0][1][0]["value"], LOGIN)
        self.assertEqual(second.calls[0][1][0]["value"], "second-login")
        self.assertNotEqual(left["cookie_header"], right["cookie_header"])
        self.assertIsNot(first.account.ctx, second.account.ctx)
        self.assertIsNot(first.account.page, second.account.page)
        self.assertIsNot(first.account._observed_scope, second.account._observed_scope)
        self.assertEqual(len(first.scope.submitted), 4)
        self.assertEqual(len(second.scope.submitted), 4)

    async def test_observed_direct_launch_does_not_inherit_ambient_proxy(self):
        host, result, launches, calls = await launch_fixture(observed=True)
        self.assertEqual(len(launches), 1)
        self.assertNotIn("proxy", launches[0])
        self.assertEqual(host.proxies, {})
        self.assertEqual([call[0] for call in calls],
                         ["mkdir", "allocation", "begin_launch", "browser_started", "initialize"])
        self.assertIs(calls[-2][1], result)
        self.assertIs(calls[-1][-1], result)

    async def test_observed_launch_keeps_only_its_explicit_or_saved_proxy(self):
        supplied, saved = "http://supplied.invalid:9101", "http://saved.invalid:9102"
        for explicit, previous, expected in ((supplied, None, supplied),
                                             (None, saved, saved), (supplied, saved, supplied)):
            with self.subTest(explicit=explicit, previous=previous):
                host, _, launches, _ = await launch_fixture(True, explicit, previous)
                self.assertEqual(launches[0]["proxy"], {"server": expected})
                self.assertEqual(host.proxies[ACCOUNT], expected)

    async def test_unmanaged_launch_preserves_ambient_proxy_fallback(self):
        _, _, launches, calls = await launch_fixture(observed=False)
        self.assertEqual(launches[0]["proxy"], {"server": "http://ambient.invalid:9100"})
        self.assertEqual([call[0] for call in calls], ["mkdir", "allocation", "initialize"])
        supplied = "http://supplied.invalid:9101"
        _, _, launches, _ = await launch_fixture(False, explicit=supplied)
        self.assertEqual(launches[0]["proxy"], {"server": supplied})


if __name__ == "__main__":
    unittest.main(verbosity=2)
