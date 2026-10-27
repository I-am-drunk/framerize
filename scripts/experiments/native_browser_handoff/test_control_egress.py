"""Paired observer controls with inert page and native-route ports."""
import asyncio
from dataclasses import replace
import json
import hashlib
import os
from pathlib import Path
import subprocess
import time
import unittest
from urllib.parse import parse_qs, urlsplit

import test_control_preparation as preparation
from control_egress import EgressAttester, NativeRouteProbe, _address, _probe_url
from control_mint import ObservationRefused


class EgressTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        fixture_type = type("Fixture", (preparation.PreparationTests,),
            {"_callCleanup": unittest.TestCase._callCleanup})
        self.fixture = fixture_type("test_preparation_binds_one_page_without_mint_or_background_work")
        await self.fixture.asyncSetUp()
        self.identity = await self.fixture.start()
        self.host = self.fixture.host
        self.page = self.fixture.ports.browsers[0].contexts[0].pages[0]
        self.calls, self.tasks = [], []
        self.entered = {key: asyncio.Event() for key in ("browser", "native")}
        self.release = {key: asyncio.Event() for key in self.entered}
        for event in self.release.values():
            event.set()
        self.changes, self.errors, self.joined = {}, set(), set()
        self.native_current = True
        self.stopped = asyncio.Event()
        self.page.evaluate = self.browser
        self.native = NativeRouteProbe(preparation.ACCOUNT, preparation.EXPECTED.user,
            None, self.native_observe, lambda: self.native_current)
        self.attester = EgressAttester("https://egress.example/ip?format=json",
            self.native, valid_seconds=5, timeout_seconds=2)

    async def step(self, kind, url, milliseconds):
        self.calls.append((kind, url, milliseconds))
        self.entered[kind].set()
        try:
            await self.release[kind].wait()
            if kind in self.errors:
                raise RuntimeError("private fixture detail")
            result = dict(version=1, url=url, ip="203.0.113.8")
            result.update(self.changes.get(kind, {}))
            return result
        finally:
            self.joined.add(kind)

    async def browser(self, script, request):
        self.assertIn("credentials: 'omit'", script)
        return await self.step("browser", request["url"], request["timeout_ms"])

    async def native_observe(self, url, milliseconds, stopped):
        self.assertIs(stopped, self.stopped)
        return await self.step("native", url, milliseconds)

    def begin(self):
        task = asyncio.create_task(self.attester(self.host, self.identity,
            preparation.EXPECTED, None, self.stopped, time.monotonic() + 5))
        self.tasks.append(task)
        return task

    async def entered_both(self):
        await asyncio.wait_for(asyncio.gather(
            *(event.wait() for event in self.entered.values())), 1)

    async def asyncTearDown(self):
        self.stopped.set()
        for event in self.release.values():
            event.set()
        await asyncio.gather(*self.tasks, return_exceptions=True)
        await self.fixture.asyncTearDown()
        self.assertTrue(self.fixture.doCleanups())

    async def test_success_joins_both_routes_and_uses_one_fresh_nonce_url(self):
        witness = await self.begin()
        self.assertTrue(witness())
        self.assertEqual(self.joined, {"browser", "native"})
        self.assertEqual(self.calls[0][1:], self.calls[1][1:])
        query = parse_qs(urlsplit(self.calls[0][1]).query)
        self.assertEqual(query["format"], ["json"])
        self.assertEqual(len(query["framerize_probe"][0]), 32)
        self.assertFalse(self.host._control_busy)
        self.assertIsNone(self.host._control_task)

    async def test_witness_expires_and_rechecks_both_owners(self):
        witness = await self.begin()
        self.assertFalse(replace(witness, expires=time.monotonic() - 1)())
        self.native_current = False
        self.assertFalse(witness())
        self.native_current = True
        self.page.url += "/changed"
        self.assertFalse(witness())
        self.page.url = preparation.EXPECTED.page_url
        self.assertTrue(witness())
        self.stopped.set()
        self.assertFalse(witness())

    async def test_different_reported_addresses_refuse_and_fence(self):
        self.changes["native"] = {"ip": "203.0.113.9"}
        with self.assertRaises(ObservationRefused):
            await self.begin()
        self.assertEqual(self.joined, {"browser", "native"})
        self.assertTrue(self.host._control_fenced)
        self.assertTrue(self.stopped.is_set())

    async def test_native_error_is_private_and_both_jobs_join(self):
        self.errors.add("native")
        with self.assertRaises(ObservationRefused) as failure:
            await self.begin()
        self.assertNotIn("private fixture", str(failure.exception))
        self.assertEqual(self.joined, {"browser", "native"})
        self.assertFalse(self.host._control_busy)

    async def test_changed_native_owner_during_observation_refuses(self):
        self.release["native"].clear()
        task = self.begin()
        await self.entered_both()
        self.native_current = False
        self.release["native"].set()
        with self.assertRaises(ObservationRefused):
            await task
        self.assertEqual(self.joined, {"browser", "native"})

    async def test_repeated_cancellation_waits_for_both_observers(self):
        for event in self.release.values():
            event.clear()
        task = self.begin()
        await self.entered_both()
        task.cancel()
        await asyncio.sleep(0)
        self.assertFalse(task.done())
        self.assertTrue(self.host._control_busy)
        self.release["browser"].set()
        task.cancel()
        await asyncio.sleep(0)
        self.assertFalse(task.done())
        self.release["native"].set()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertEqual(self.joined, {"browser", "native"})
        self.assertTrue(self.host._control_fenced)
        self.assertFalse(self.host._control_busy)
        self.assertIsNone(self.host._control_task)

    async def test_deadline_waits_for_late_native_settlement(self):
        self.attester = replace(self.attester, timeout_seconds=0.04)
        self.release["native"].clear()
        task = self.begin()
        await self.entered_both()
        await asyncio.sleep(0.06)
        self.assertTrue(self.stopped.is_set())
        self.assertFalse(task.done())
        self.release["native"].set()
        with self.assertRaises(ObservationRefused):
            await task
        self.assertEqual(self.joined, {"browser", "native"})

    async def test_host_close_waits_for_native_observation(self):
        self.release["native"].clear()
        task = self.begin()
        await self.entered_both()
        closed = asyncio.create_task(self.host.close())
        self.tasks.append(closed)
        await asyncio.sleep(0)
        self.assertFalse(closed.done())
        self.assertNotIn("native", self.joined)
        self.release["native"].set()
        with self.assertRaises(ObservationRefused):
            await task
        await closed
        self.assertEqual(self.joined, {"browser", "native"})

    async def test_wrong_route_bindings_refuse_before_network_work(self):
        for change in (dict(account="another-account"), dict(user="another-user"),
                       dict(proxy="http://proxy.invalid"), dict(current=lambda: False)):
            with self.subTest(fields=list(change)):
                self.attester = replace(self.attester, native=replace(self.native, **change))
                with self.assertRaises(ObservationRefused):
                    await self.begin()
        self.assertEqual(self.calls, [])
        self.assertFalse(self.host._control_busy)

    async def test_busy_host_refuses_without_an_observation(self):
        self.host._control_busy = True
        try:
            with self.assertRaises(ObservationRefused):
                await self.begin()
            self.assertEqual(self.calls, [])
        finally:
            self.host._control_busy = False

    async def test_invalid_limits_refuse_without_an_observation(self):
        original = self.attester
        for change in (dict(valid_seconds=0), dict(valid_seconds=float("inf")),
                       dict(timeout_seconds=True), dict(timeout_seconds=31)):
            self.attester = replace(original, **change)
            with self.assertRaises(ObservationRefused):
                await self.begin()
        self.assertEqual(self.calls, [])


class ValidationTests(unittest.TestCase):
    def test_endpoint_requires_credential_free_https_without_fragment(self):
        for endpoint in ("http://example.com/ip", "https://user:pass@example.com/ip",
                         "https://example.com/ip#fragment", "https://example.com:bad/ip",
                         "https://example.com/\n", "https://example.com/\\ip",
                         "https://example.com/?framerize_probe=old",
                         "https://example.com/?framerize_probe",
                         "https://example.com/?framerize_probe=",
                         "https://example.com/?%66ramerize_probe="):
            with self.subTest(endpoint=endpoint), self.assertRaises(ObservationRefused):
                _probe_url(endpoint)

    def test_nonce_urls_preserve_query_and_differ_per_observation(self):
        raw = "format=json&flag&blank=&encoded=%2f%2B+&repeat=1&repeat=2"
        first, second = (_probe_url("https://EXAMPLE.com:443?" + raw) for _ in range(2))
        self.assertNotEqual(first, second)
        self.assertEqual(urlsplit(first).netloc, "example.com")
        self.assertEqual(urlsplit(first).path, "/")
        self.assertEqual(parse_qs(urlsplit(first).query)["format"], ["json"])
        for value in (first, second):
            query = urlsplit(value).query
            self.assertEqual(query.rsplit("&", 1)[0], raw)
            fields = parse_qs(query, keep_blank_values=True)
            self.assertEqual(len(fields["framerize_probe"]), 1)
            self.assertEqual(len(fields["framerize_probe"][0]), 32)

    def test_replies_require_exact_url_schema_and_valid_ip(self):
        url = "https://example.com/?framerize_probe=fixture"
        base = dict(version=1, url=url, ip="203.0.113.8")
        for change in (dict(version=True), dict(url=url + "x"), dict(extra=True),
                       dict(ip="not-an-ip"), dict(ip="fe80::1%eth0"), dict(ip=None)):
            with self.subTest(fields=list(change)), self.assertRaises(ObservationRefused):
                _address(dict(base, **change), url)
        self.assertEqual(_address(base, url), "203.0.113.8")

    def test_ipv6_comparison_uses_canonical_address(self):
        self.assertEqual(_address(dict(version=1, url="x", ip="2001:0db8:0:0::1"), "x"),
            "2001:db8::1")


class ScriptTests(unittest.TestCase):
    def check_modes(self, modes):
        node = Path(os.environ["FRAMERIZE_TEST_CONTROL_NODE"])
        self.assertTrue(node.is_absolute())
        self.assertEqual(hashlib.sha256(node.read_bytes()).hexdigest(),
            os.environ["FRAMERIZE_TEST_CONTROL_NODE_SHA256"])
        result = subprocess.run([str(node), "-e", SCRIPT,
            str(Path(__file__).with_name("control_egress.js")), json.dumps(modes)],
            capture_output=True, text=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(json.loads(result.stdout), modes)

    def test_page_fetch_omits_credentials_and_joins_body_cleanup(self):
        self.check_modes(["ok"])

    def test_page_fetch_rejects_redirects_status_schema_and_size(self):
        self.check_modes(["redirect", "status", "duplicate", "utf8", "oversized",
            "empty", "content-type", "read-limit"])

    def test_page_fetch_timeout_and_failed_cleanup_refuse(self):
        self.check_modes(["stall", "cleanup-error", "release-error", "read-error"])


SCRIPT = r"""
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const source = fs.readFileSync(process.argv[1], 'utf8');
const modes = JSON.parse(process.argv[2]);
async function run(mode) {
  let cancelled = false, released = false, options, reads = 0;
  let body = new TextEncoder().encode('{"ip":"203.0.113.8"}');
  if (mode === 'duplicate') body = new TextEncoder().encode('{"ip":"1.1.1.1","ip":"2.2.2.2"}');
  if (mode === 'utf8') body = new Uint8Array([255]);
  if (mode === 'oversized') body = new Uint8Array(1025);
  if (mode === 'empty') body = new Uint8Array();
  const cancel = async () => {
    await new Promise(resolve => setImmediate(resolve));
    cancelled = true;
    if (mode === 'cleanup-error') throw new Error('cleanup refused');
  };
  const reader = {
    async read() {
      if (mode === 'stall') return await new Promise((resolve, reject) =>
        options.signal.addEventListener('abort', () => reject(new Error('aborted')), {once:true}));
      if (mode === 'read-error') throw new Error('read refused');
      if (mode === 'read-limit') return {done:false, value:new Uint8Array()};
      return reads++ === 0 ? {done:false, value:body} : {done:true};
    },
    cancel,
    releaseLock() {
      released = true;
      if (mode === 'release-error') throw new Error('release refused');
    }
  };
  const fetch = async (url, supplied) => {
    options = supplied;
    assert.equal(options.credentials, 'omit');
    assert.equal(options.referrerPolicy, 'no-referrer');
    assert.equal(options.redirect, 'error');
    assert.equal(options.cache, 'no-store');
    assert.equal(options.method, 'GET');
    return {status:mode === 'status' ? 403 : 200, url,
      redirected:mode === 'redirect',
      headers:{get:() => mode === 'content-type' ? 'text/html' : 'application/json'},
      body:{getReader:() => reader, cancel}};
  };
  const timers = new Set();
  const context = {URL, AbortController, TextDecoder, Uint8Array, fetch,
    setTimeout(fn, ms) {const id=setTimeout(fn, ms); timers.add(id); return id;},
    clearTimeout(id) {timers.delete(id); clearTimeout(id);}};
  const observe = vm.runInNewContext('(' + source + ')', context);
  let result, failed = false;
  try {
    result = await observe({url:'https://egress.example/ip?probe=fixed',
      timeout_ms:mode === 'stall' ? 5 : 1000});
  } catch {failed = true;}
  assert.equal(failed, mode !== 'ok', mode);
  if (!failed) assert.equal(result.ip, '203.0.113.8');
  assert.equal(cancelled, true, mode + ': joined cancel');
  if (!['redirect', 'status', 'content-type'].includes(mode))
    assert.equal(released, true, mode + ': released reader');
  assert.equal(options.signal.aborted, true);
  assert.equal(timers.size, 0);
}
(async () => {
  for (const mode of modes) await run(mode);
  process.stdout.write(JSON.stringify(modes));
})().catch(error => {console.error(error); process.exitCode = 1;});
"""
