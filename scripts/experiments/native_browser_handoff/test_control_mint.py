"""Offline observation checks. No browser, credentials or network are used."""
import asyncio
from dataclasses import replace
import json
from pathlib import Path
import shutil
import subprocess
import time
from types import SimpleNamespace
import unittest

from control_mint import ExpectedContext, ObservationRefused, observe


EXPECTED = ExpectedContext("fixture-user", "fixture-device", "fixture-session",
    "https://framer.com/projects/fixture",
    "https://app.framerstatic.com/fixture-wrapper.mjs", "/ai/v3/openai/responses/")
PROFILE = "https://api.framer.com/web/users/me"
POST = "https://api.framer.com" + EXPECTED.post_path


class Page:
    def __init__(self):
        self.context = SimpleNamespace(cookies=self.cookies)
        self.url, self.closed, self.owned = EXPECTED.page_url, False, True
        self.calls, self.values = [], {}
        self.after_cookie = lambda url: None
        self.after_evaluate = lambda: None
        self.result = {"version": 1, "token": "fixture-token", "matched":
            dict(user=True, device=True, page=True, wrapper=True)}

    def is_closed(self):
        return self.closed

    async def cookies(self, url):
        self.calls.append(url)
        self.after_cookie(url)
        return self.values.get(url, [dict(name="session", value=EXPECTED.session),
            dict(name="__cuid", value=EXPECTED.device)])

    async def evaluate(self, source, request):
        self.calls.append("evaluate")
        assert "session" not in request
        self.after_evaluate()
        return self.result


async def rpc(fn, *args):
    return await fn(*args)


class ContextTests(unittest.IsolatedAsyncioTestCase):
    async def run_observation(self, page, expected=EXPECTED, deadline=None, call=rpc):
        return await observe(page, expected, call, lambda: page.owned,
            time.monotonic() + 30 if deadline is None else deadline)

    async def test_exact_context_checks_both_urls_before_and_after_one_call(self):
        page = Page()
        result = await self.run_observation(page)
        self.assertEqual(page.calls, [PROFILE, POST, "evaluate", PROFILE, POST])
        self.assertEqual(result.token, "fixture-token")
        self.assertLessEqual(result.invocation_started, result.received_at)
        self.assertNotIn(result.token, repr(result))

    async def test_path_scoped_mismatch_or_duplicate_refuses_before_mint(self):
        session = dict(name="session", value=EXPECTED.session)
        device = dict(name="__cuid", value=EXPECTED.device)
        invalid = ([], [dict(session, value="other"), device],
            [session, dict(device, value="other")], [session, device, session],
            [session, device, device])
        for url in (PROFILE, POST):
            for cookies in invalid:
                with self.subTest(url=url, cookies=cookies):
                    page = Page()
                    page.values[url] = cookies
                    with self.assertRaises(ObservationRefused):
                        await self.run_observation(page)
                    self.assertNotIn("evaluate", page.calls)

    async def test_either_cookie_scope_changing_during_mint_refuses_token(self):
        for url in (PROFILE, POST):
            with self.subTest(url=url):
                page = Page()
                page.after_evaluate = lambda: page.values.update({url: []})
                with self.assertRaises(ObservationRefused):
                    await self.run_observation(page)
                self.assertEqual(page.calls.count("evaluate"), 1)

    async def test_rpc_error_is_payload_free_and_does_not_retry(self):
        page = Page()
        async def broken(fn, *args):
            raise RuntimeError("fixture-private-detail")
        with self.assertRaises(ObservationRefused) as caught:
            await self.run_observation(page, call=broken)
        self.assertEqual(str(caught.exception), "browser mint observation refused")
        self.assertTrue(caught.exception.__suppress_context__)
        self.assertEqual(page.calls, [])

    async def test_cancellation_propagates_and_caller_rpc_settles(self):
        page, entered, settled = Page(), asyncio.Event(), asyncio.Event()
        async def held(fn, *args):
            if fn.__name__ != "evaluate":
                return await fn(*args)
            entered.set()
            try:
                await asyncio.Future()
            finally:
                settled.set()
        task = asyncio.create_task(self.run_observation(page, call=held))
        try:
            await asyncio.wait_for(entered.wait(), 1)
        finally:
            task.cancel()
            with self.assertRaises(asyncio.CancelledError):
                await task
        self.assertTrue(settled.is_set())
        self.assertFalse(page.closed)
        self.assertEqual(page.calls, [PROFILE, POST])

    async def test_owner_page_or_context_change_refuses_at_each_boundary(self):
        for attr, value in (("owned", False), ("closed", True),
                ("url", "https://framer.com/other"), ("context", object())):
            for boundary in ("cookie", "evaluate"):
                with self.subTest(attr=attr, boundary=boundary):
                    page = Page()
                    mutate = lambda *args: setattr(page, attr, value)
                    setattr(page, "after_" + boundary, mutate)
                    with self.assertRaises(ObservationRefused):
                        await self.run_observation(page)
                    self.assertEqual(page.calls.count("evaluate"), int(boundary == "evaluate"))

    async def test_invalid_inputs_refuse_without_rpc(self):
        for expected in (replace(EXPECTED, session=""),
                replace(EXPECTED, page_url="https://other.invalid/"),
                replace(EXPECTED, wrapper_url=EXPECTED.wrapper_url + "#fragment"),
                replace(EXPECTED, post_path="/web/users/me")):
            with self.subTest(expected=repr(expected)):
                page = Page()
                with self.assertRaises(ObservationRefused):
                    await self.run_observation(page, expected)
                self.assertEqual(page.calls, [])
        for deadline in (True, float("nan"), float("inf"), 0, time.monotonic() + 200):
            with self.subTest(deadline=deadline):
                page = Page()
                with self.assertRaises(ObservationRefused):
                    await self.run_observation(page, deadline=deadline)
                self.assertEqual(page.calls, [])

    async def test_malformed_or_oversized_results_refuse_without_retry(self):
        for change in ({"version": True}, {"token": ""}, {"token": "é" * 8193},
                {"matched": {}}, {"extra": "unexpected"}, {"token": None}):
            with self.subTest(change=list(change)):
                page = Page()
                page.result.update(change)
                with self.assertRaises(ObservationRefused):
                    await self.run_observation(page)
                self.assertEqual(page.calls.count("evaluate"), 1)


NODE_FIXTURE = r"""
const assert = require('node:assert/strict');
const vm = require('node:vm');
const fs = require('node:fs');
const {TextEncoder, TextDecoder} = require('node:util');
const {ReadableStream} = require('node:stream/web');
const source = fs.readFileSync(process.argv[1], 'utf8');
const request = JSON.parse(process.argv[2]);
const s = {mints: 0, fetches: 0, cancels: 0, status: 200, close: true,
  raw: new TextEncoder().encode(JSON.stringify({id: request.user})),
  token: 'fixture-token'};
const window = {};
const moduleObject = {a: async function() {
  assert.equal(this, moduleObject);
  s.mints++;
  if (s.afterMint) await s.afterMint();
  return s.token;
}};
window.__fzResidentWrapper = Object.freeze({module: moduleObject,
  callable: moduleObject.a, entry: 'a', url: request.wrapper_url, realm: window});
const location = {origin: 'https://framer.com', href: request.page_url};
const document = {cookie: '__cuid=' + request.device};
const localStorage = {getItem: () => request.device};
const fetch = async (url, options) => {
  s.fetches++;
  assert.equal(url, 'https://api.framer.com/web/users/me');
  assert.equal(options.credentials, 'include');
  assert.equal(options.redirect, 'error');
  assert.equal(options.cache, 'no-store');
  s.stream = new ReadableStream({start(controller) {
    if (s.raw !== null) controller.enqueue(s.raw);
    if (s.close) controller.close();
  }, cancel() {
    s.cancels++;
    if (s.cancel) return s.cancel();
  }});
  if (s.afterFetch) s.afterFetch();
  return {status: s.status, body: s.noBody ? null : s.stream};
};
const run = () => vm.runInNewContext('(' + source + ')', {
  window, location, document, localStorage, fetch, TextEncoder, TextDecoder,
})(request);
const refused = promise => assert.rejects(promise,
  error => error.message === 'browser mint observation refused');
"""


class PageScriptTests(unittest.TestCase):
    def js(self, case):
        node = shutil.which("node")
        self.assertIsNotNone(node, "Node is required for the actual page-script checks")
        script = NODE_FIXTURE + "\n(async () => {\n" + case
        script += "\nconsole.log('ok');\n})().catch(e => { console.error(e); process.exitCode = 1; });"
        request = {key: getattr(EXPECTED, key) for key in
            ("user", "device", "page_url", "wrapper_url")}
        result = subprocess.run([node, "-e", script,
            str(Path(__file__).with_name("control_mint.js")), json.dumps(request)],
            capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.strip(), "ok", result.stderr)

    def test_single_observed_wrapper_call_and_profile_body_join(self):
        self.js("""
          const result = await run();
          assert.equal(result.token, s.token);
          assert.equal(s.mints, 1); assert.equal(s.fetches, 1);
          assert.equal(s.stream.locked, false); assert.equal(s.cancels, 0);
          assert.deepEqual(Object.values(result.matched), [true, true, true, true]);
        """)

    def test_non_200_waits_for_body_cancellation_before_refusal(self):
        self.js("""
          s.status = 403; s.raw = null; s.close = false;
          let entered, release, finished = false;
          const cancelling = new Promise(resolve => { entered = resolve; });
          const barrier = new Promise(resolve => { release = resolve; });
          s.cancel = () => { entered(); return barrier; };
          const pending = refused(run()).then(() => { finished = true; });
          await cancelling;
          assert.equal(finished, false); assert.equal(s.mints, 0);
          release(); await pending;
          assert.equal(s.cancels, 1); assert.equal(s.fetches, 1);
        """)

    def test_cancel_error_remains_payload_free_without_mint(self):
        self.js("""
          s.status = 500; s.close = false;
          s.cancel = async () => { throw new Error('fixture-private-detail'); };
          await refused(run());
          assert.equal(s.mints, 0); assert.equal(s.cancels, 1);
        """)

    def test_missing_body_or_wrong_profile_never_mints(self):
        self.js("""
          s.noBody = true; await refused(run());
          s.noBody = false;
          s.raw = new TextEncoder().encode('{"id":"another-user"}');
          await refused(run());
          assert.equal(s.mints, 0); assert.equal(s.stream.locked, false);
        """)

    def test_invalid_utf8_and_oversized_body_cancel_and_release_reader(self):
        self.js("""
          s.close = false; s.raw = new Uint8Array([255]);
          await refused(run());
          assert.equal(s.cancels, 1); assert.equal(s.stream.locked, false);
          s.raw = new Uint8Array(1024 * 1024 + 1);
          await refused(run());
          assert.equal(s.cancels, 2); assert.equal(s.stream.locked, false);
          assert.equal(s.mints, 0);
        """)

    def test_page_device_and_wrapper_changes_refuse_before_or_after_mint(self):
        mutations = ["location.href += '?changed'",
            "document.cookie = '__cuid=other'",
            "localStorage.getItem = () => 'other'",
            "window.__fzResidentWrapper = Object.freeze({...window.__fzResidentWrapper})",
            "moduleObject.a = () => 'other'"]
        for phase in ("afterFetch", "afterMint"):
            for mutation in mutations:
                with self.subTest(phase=phase, mutation=mutation):
                    self.js("s." + phase + " = () => {" + mutation + "};"
                        "await refused(run()); assert.equal(s.mints, "
                        + str(int(phase == "afterMint")) + ");")

    def test_invalid_initial_wrapper_refuses_before_profile_fetch(self):
        self.js("""
          window.__fzResidentWrapper = {...window.__fzResidentWrapper};
          await refused(run());
          assert.equal(s.fetches, 0); assert.equal(s.mints, 0);
        """)

    def test_invalid_json_releases_reader_without_mint(self):
        self.js("""
          s.raw = new TextEncoder().encode('{incomplete');
          await refused(run());
          assert.equal(s.stream.locked, false); assert.equal(s.mints, 0);
        """)

    def test_token_byte_bound_and_wrapper_error_never_retry(self):
        self.js("""
          s.token = 'é'.repeat(8192);
          await run(); assert.equal(s.mints, 1);
          s.token += 'é';
          await refused(run()); assert.equal(s.mints, 2);
          s.token = '';
          await refused(run()); assert.equal(s.mints, 3);
          s.afterMint = () => { throw new Error('fixture-private-detail'); };
          await refused(run()); assert.equal(s.mints, 4);
        """)


if __name__ == "__main__":
    unittest.main()
