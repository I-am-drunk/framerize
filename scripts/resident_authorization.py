"""Private, source-only resident control; importing this file activates nothing.

Root must bind host.resident_application_generation from the trusted daemon,
retain one ResidentControl(host), and dispatch command(request) outside the
ordinary activity gate. Host.generation and lifecycle.activity remain owners.
No token is cached in a completion or returned by a repeated acquire.

command accepts version=1, action, account, application_generation (positive
u64), host_generation, account_incarnation (positive u64), and lease UUID.
Acquire adds project/path/wrapper_url/wrapper_export/cookie_header/budget_ms.
The input header supplies the daemon's original session, not the resident jar.
Finish adds native_state='settled', admission='resume' or 'fenced'. Discard
adds native_state='not_started' or 'unknown'. Optional id/cmd are not echoed.
Acquire returns the six identity fields, state='acquired', browser_settled,
token/current cookie_header/page_url/device/effective_proxy (None is direct).
Finish/discard return only identity, state='released' or 'fenced', and
browser_settled. Their retained completion has no deadline-based release.

Version 2 adds a provenance UUID to acquire and every context-bearing reply.
It acquires a read-only snapshot and private wrapper handle, with no token.
Baseline takes identity/action/bearer/body, mints once and consumes one fetch.
Only an accepted, unchanged baseline enables identity/action mint_native once.
Both mints use existing host spacing, device cooldowns and request accounting.
The original acquire deadline spans every phase; waiter loss fences admission
while raw RPC and phase owners finish. Final receipts keep only identity,
state and browser_settled. The retained JS handle is disposed without closing
the browser. No live activation or validation is claimed by this source.

The selected HTTP-200 page may be /login. Project authority comes solely
from the completed response provenance. No editor readiness is consulted.
Wrapper metadata must be a frozen data record created at the actual cache
installation: {module, callable, url, entry, realm: window}. No metadata is
inferred from an old cache. Root must gate all ordinary host activity and
retain the native operation until its settlement; this module proves neither
native settlement nor page-script quiescence beyond its observed barriers.
"""
import asyncio
import hashlib
import json
import math
import re
import time
import uuid
from urllib.parse import urlsplit

from contextual_authorization import cookies_for_url, snapshot_header, login_cookie
from contextual_authorization import baseline_result, comparison_json
from resident_activity import ActivityLedger, ResidentRefused

COMMAND = 'resident_authorization_once'
LIMIT = 16 * 1024
MAX_BUDGET_MS = 45_000
BODY_LIMIT = 64 * 1024
IDENTITY = frozenset(('version', 'account', 'application_generation',
    'host_generation', 'account_incarnation', 'lease'))
# Exact inference routes from crates/framer-api/src/routes; no utility routes.
ROUTES = frozenset(('/ai/v3/responses/', '/ai/v3/messages/', '/ai/v3/chat/',
    '/ai/v3/openai/responses/', '/ai/v3/anthropic/messages/'))
_FAILED = object()


def _refuse():
    raise ResidentRefused('resident authorization refused') from None


def _text(value, limit):
    return (type(value) is str and 0 < len(value) <= limit
        and len(value.encode('utf-8', 'strict')) <= limit
        and not any(ord(c) < 32 or 127 <= ord(c) <= 159 for c in value))


def _same_cookie_snapshot(actual, expected):
    # Both headers were validated by snapshot_header. Compare stable name order
    # while retaining the original header for transport and duplicate precedence.
    def ordered(snapshot):
        header, device = snapshot
        return device, sorted(header.split('; '), key=lambda part: part.partition('=')[0])
    return ordered(actual) == ordered(expected)


def _positive(value):
    return type(value) is int and 0 < value < 2 ** 64


def _uuid(value):
    return (type(value) is str and str(uuid.UUID(value)) == value
        and uuid.UUID(value).int != 0)


def validate_request(request):
    """Validate the private wire frame before ownership or browser work."""
    try:
        if type(request) is not dict:
            _refuse()
        action = request.get('action')
        extras = {'acquire': {'project', 'path', 'wrapper_url', 'wrapper_export',
                             'cookie_header', 'budget_ms'},
                  'finish': {'native_state', 'admission'},
                  'discard': {'native_state'}}
        if type(request.get('version')) is int and request['version'] == 2:
            extras.update(baseline={'bearer', 'body'}, mint_native=set())
            extras['acquire'] = extras['acquire'] | {'provenance'}
        if type(action) is not str or action not in extras:
            _refuse()
        required = IDENTITY | {'action'} | extras[action]
        if not required <= request.keys() or request.keys() - required - {'id', 'cmd'}:
            _refuse()
        if 'cmd' in request and (type(request['cmd']) is not str or request['cmd'] != COMMAND):
            _refuse()
        if 'id' in request and (type(request['id']) is not int
                               or not 0 <= request['id'] < 2 ** 64):
            _refuse()
        if (type(request['version']) is not int or request['version'] not in (1, 2)
                or not _uuid(request['account']) or not _uuid(request['lease'])
                or not _positive(request['application_generation'])
                or not _positive(request['account_incarnation'])
                or not _text(request['host_generation'], 128)):
            _refuse()
        if action == 'acquire':
            _validate_acquire(request)
            if request['version'] == 2 and not _uuid(request['provenance']):
                _refuse()
        elif action == 'baseline':
            return _validate_baseline(request)
        elif action == 'mint_native':
            pass
        elif action == 'finish':
            if (type(request['native_state']) is not str or request['native_state'] != 'settled'
                    or type(request['admission']) is not str
                    or request['admission'] not in ('resume', 'fenced')):
                _refuse()
        elif type(request['native_state']) is not str or request['native_state'] not in ('not_started', 'unknown'):
            _refuse()
        return dict(request)
    except (ValueError, TypeError, AttributeError, OverflowError):
        _refuse()


def _validate_baseline(req):
    if not _text(req['bearer'], LIMIT) or type(req['body']) is not dict:
        _refuse()
    def plain(value, depth=0):
        if depth > 64:
            _refuse()
        if type(value) is dict:
            if any(type(k) is not str or k == 'castleToken' for k in value):
                _refuse()
            for item in value.values():
                plain(item, depth + 1)
        elif type(value) is list:
            for item in value:
                plain(item, depth + 1)
        elif type(value) in (int, float):
            # Match the resident Rust body guard before browser JSON.parse.
            # Exact type checks keep JSON booleans distinct from numbers.
            if not -(2 ** 53 - 1) <= value <= 2 ** 53 - 1:
                _refuse()
        elif value is not None and type(value) not in (str, int, float, bool):
            _refuse()
    plain(req['body'])
    body = json.dumps(req['body'], ensure_ascii=False, allow_nan=False,
                      separators=(',', ':'))
    if len(body.encode('utf-8', 'strict')) > BODY_LIMIT:
        _refuse()
    # Own a deep wire copy; a caller cannot change the body after admission.
    return dict(req, body=json.loads(body))


def _validate_acquire(req):
    if (type(req['project']) is not str
            or not re.fullmatch(r'[A-Za-z0-9_-]{1,128}', req['project'])
            or type(req['path']) is not str or req['path'] not in ROUTES
            or type(req['budget_ms']) is not int
            or not 1 <= req['budget_ms'] <= MAX_BUDGET_MS
            or not _text(req['wrapper_url'], 8192)
            or not re.fullmatch(r'https://app[.]framerstatic[.]com/chunk-[A-Z0-9]{8}'
                                r'[.]mjs(?:[?]cb=[0-9]{1,20})?', req['wrapper_url'])
            or type(req['wrapper_export']) is not str
            or not re.fullmatch(r'[A-Za-z_$][A-Za-z0-9_$]{0,127}', req['wrapper_export'])):
        _refuse()
    login_cookie(req['cookie_header'])


def _valid_page_url(url):
    if not _text(url, 4096) or any(c.isspace() for c in url):
        return False
    parsed = urlsplit(url)
    return parsed.scheme == 'https' and parsed.netloc in ('framer.com', 'framer.com:443')


# Plain evaluate, deliberately the same realm selector as normal MINT_JS.
# The only SDK operation is one call to the already cached wrapper callable.
RESIDENT_MINT_JS = r"""async input => {
  const fail = code => ({ok: false, code});
  let stage = 'resident_context';
  try {
    const realm = window, doc = document, storage = window.localStorage;
    const record = Object.getOwnPropertyDescriptor(window, '__fzResidentWrapper')?.value;
    const module = Object.getOwnPropertyDescriptor(window, '__fzCastleM')?.value;
    const field = (object, key) => Object.getOwnPropertyDescriptor(object, key)?.value;
    const callable = record && field(record, 'callable');
    const current = () => window === realm && document === doc
      && window.localStorage === storage && location.href === input.page_url
      && storage.getItem('__cuid') === input.device;
    const cached = () => record && Object.isFrozen(record)
      && Object.getOwnPropertyDescriptor(window, '__fzResidentWrapper')?.value === record
      && Object.getOwnPropertyDescriptor(window, '__fzCastleM')?.value === module
      && field(record, 'module') === module && field(record, 'realm') === realm
      && field(record, 'url') === input.wrapper_url
      && field(record, 'entry') === input.wrapper_export
      && typeof callable === 'function' && field(record, 'callable') === callable
      && field(module, input.wrapper_export) === callable;
    if (!current()) return fail(stage);
    stage = 'resident_cache';
    if (!cached()) return fail(stage);
    stage = 'resident_call';
    let token = await Reflect.apply(callable, module, []);
    stage = 'resident_context';
    if (!current()) { token = null; return fail(stage); }
    stage = 'resident_cache';
    if (!cached()) { token = null; return fail(stage); }
    stage = 'resident_token';
    if (typeof token !== 'string' || !token.length || token.length > 16384)
      return fail(stage);
    let bytes = 0;
    for (let i = 0; i < token.length; i++) {
      const c = token.charCodeAt(i);
      if (c < 32 || (c >= 127 && c <= 159)) return fail(stage);
      if (c >= 0xD800 && c <= 0xDBFF) {
        const low = token.charCodeAt(++i);
        if (!(low >= 0xDC00 && low <= 0xDFFF)) return fail(stage);
        bytes += 4;
      } else if (c >= 0xDC00 && c <= 0xDFFF) return fail(stage);
      else bytes += c < 128 ? 1 : c < 2048 ? 2 : 3;
      if (bytes > 16384) return fail(stage);
    }
    return {ok: true, token};
  } catch (_) { return fail(stage); }
}"""


class _Binding:
    """Captured live objects, never get-or-create or a configured proxy guess."""
    def __init__(self, control, req):
        self.control = control
        self.req = {key: req[key] for key in IDENTITY | {'project'}}
        host = control.host
        self.accounts, self.lifecycles = host.accounts, host.lifecycles
        self.acc = self.accounts.get(req['account'])
        self.life = self.lifecycles.get(req['account'])
        if self.acc is None or self.life is None:
            _refuse()
        self.ledger = self.life.activity
        self.evidence = self.acc.completed_browser_http200
        if type(self.ledger) is not ActivityLedger or self.evidence is None:
            _refuse()
        self.page, self.context = self.evidence.page, self.evidence.context
        self.browser, self.owner = self.evidence.browser, self.evidence.owner
        self.launch, self.session = self.evidence.launch, self.acc.session_cookie
        self.page_url = self.page.url
        self.check()

    def check(self):
        host, req = self.control.host, self.req
        if (host.accounts is not self.accounts or host.lifecycles is not self.lifecycles
                or self.accounts.get(req['account']) is not self.acc
                or self.lifecycles.get(req['account']) is not self.life
                or host.generation != req['host_generation']
                or host.generation != self.control.host_generation
                or host.resident_application_generation != req['application_generation']
                or not _positive(host.resident_application_generation)
                or host.resident_application_generation != self.control.application_generation
                or host._closed or host._observed is not None
                or self.acc.lifecycle is not self.life or self.life.activity is not self.ledger
                or self.life.resident_uncertain is not None
                or self.life.incarnation != req['account_incarnation']
                or not _positive(self.life.incarnation)
                or self.life.active_commands != 0 or self.acc.active_requests != 0
                or self.life.close_task is not None or self.acc.closing
                or self.acc.completed_browser_http200 is not self.evidence
                or not self.evidence.current() or self.evidence.host is not host
                or self.evidence.account != req['account'] or self.evidence.acc is not self.acc
                or self.evidence.life is not self.life or self.evidence.page is not self.page
                or self.evidence.context is not self.context or self.evidence.browser is not self.browser
                or self.evidence.owner is not self.owner or self.evidence.project != req['project']
                or self.evidence.launch is not self.launch or self.acc.resident_launch is not self.launch
                or type(self.launch) is not tuple or len(self.launch) != 4
                or self.launch[0] is not self.browser or self.launch[1] != req['account_incarnation']
                or self.launch[3] is not False
                or (self.launch[2] is not None and not _text(self.launch[2], LIMIT))
                or type(self.session) is not tuple or len(self.session) != 2
                or self.session[0] != 'session' or not _text(self.session[1], LIMIT)
                or self.acc.session_cookie != self.session or self.page.url != self.page_url
                or not _valid_page_url(self.page_url)):
            _refuse()


class ResidentControl:
    """One trusted host owner. No endpoint, background task or browser on init."""
    def __init__(self, host):
        self.host = host
        self.application_generation = getattr(host, 'resident_application_generation', None)
        self.host_generation = getattr(host, 'generation', None)
        if not _positive(self.application_generation) or not _text(self.host_generation, 128):
            _refuse()
        self._owners = set()

    def _retain(self, coroutine):
        task = asyncio.create_task(coroutine)
        self._owners.add(task)
        def completed(done):
            self._owners.discard(done)
            if not done.cancelled():
                done.exception()
        task.add_done_callback(completed)
        return task

    async def command(self, request):
        """Private request/reply; cancellation never cancels a browser RPC."""
        req = validate_request(request)
        try:
            if req['action'] == 'acquire':
                if req['version'] == 2:
                    binding = _CrossoverBinding(self, req)
                    lease = binding.ledger.acquire(req['lease'])
                    op = _CrossoverOperation(self, req, binding, lease)
                    lease._resident_operation = op
                    return await op.phase_call(req)
                return await self._acquire(req)
            op = self._lookup(req)
            if req['action'] in ('baseline', 'mint_native'):
                return await op.phase_call(req)
            op.close(req['native_state'], req.get('admission', 'resume') == 'resume')
            return dict(await asyncio.shield(op.completion))
        except Exception:
            _refuse()

    async def _acquire(self, req):
        binding = _Binding(self, req)
        # No await between checking live bindings and reserving the ledger.
        lease = binding.ledger.acquire(req['lease'])
        op = _Operation(self, req, binding, lease)
        lease._resident_operation = op
        op.owner = self._retain(op.run())
        try:
            await asyncio.wait_for(asyncio.shield(op.owner), req['budget_ms'] / 1000)
            op.admitted()
            if op.closed or op.payload is None:
                _refuse()
            payload, op.payload = op.payload, None
            return payload
        except BaseException:
            op.close('unknown', False)
            raise

    def _lookup(self, req):
        life = self.host.lifecycles.get(req['account'])
        if life is None or type(life.activity) is not ActivityLedger:
            _refuse()
        lease = life.activity.leases.get(req['lease'])
        op = getattr(lease, '_resident_operation', None)
        if op is None or op.control is not self or op.identity != {k: req[k] for k in IDENTITY}:
            _refuse()
        if op.binding is not None:
            try:
                op.binding.check()
            except Exception:
                op.close('unknown', False)
                _refuse()
        elif (self.host.generation != req['host_generation']
              or self.host.resident_application_generation != req['application_generation']
              or not _positive(self.host.resident_application_generation)
              or life.incarnation != req['account_incarnation']
              or getattr(self.host.accounts.get(req['account']), 'lifecycle', None) is not life):
            _refuse()
        return op


class _Operation:
    def __init__(self, control, req, binding, lease):
        self.control, self.binding, self.lease = control, binding, lease
        self.identity = {key: req[key] for key in IDENTITY}
        self.spec, self.ledger = req, binding.ledger
        self.deadline = asyncio.get_running_loop().time() + req['budget_ms'] / 1000
        self.owner = self.completion = self.payload = None
        self.closed = self.failed = self.browser_unknown = False

    def fence(self):
        self.ledger.fenced = True

    def admitted(self):
        if (self.closed or self.ledger.fenced or self.ledger.current is not self.lease
                or asyncio.get_running_loop().time() >= self.deadline):
            _refuse()
        self.binding.check()

    async def rpc(self, call, *args):
        self.admitted()
        async def actual():
            try:
                return await call(*args)
            except BaseException:
                # An RPC error/cancellation does not prove remote settlement.
                self.browser_unknown = True
                self.fence()
                return _FAILED
        result = await self.lease.rpc(actual)
        self.admitted()
        if result is _FAILED:
            _refuse()
        return result

    async def snapshot(self):
        url = 'https://api.framer.com' + self.spec['path']
        rows = cookies_for_url(await self.rpc(self.binding.context.cookies, url), url)
        header = snapshot_header(rows)
        devices = [row['value'] for row in rows if row.get('name') == '__cuid']
        if (len(devices) != 1
                or not _text(devices[0], 256)
                or any(ord(c) > 126 or c in ';,\\" ' for c in devices[0])
                or login_cookie(header) != self.binding.session[1]
                or login_cookie(header) != login_cookie(self.spec['cookie_header'])
                or (self.binding.owner is not self.binding.acc
                    and self.binding.owner.id != devices[0])):
            _refuse()
        return header, devices[0]

    async def run(self):
        try:
            before = await self.snapshot()
            args = {key: self.spec[key] for key in ('wrapper_url', 'wrapper_export')}
            args.update(page_url=self.binding.page_url, device=before[1])
            result = await self.rpc(self.binding.page.evaluate, RESIDENT_MINT_JS, args)
            if (type(result) is not dict or set(result) != {'ok', 'token'}
                    or result['ok'] is not True or not _text(result['token'], LIMIT)):
                _refuse()
            if await self.snapshot() != before:
                _refuse()
            self.admitted()
            self.payload = dict(self.identity, state='acquired', browser_settled=True,
                token=result['token'], cookie_header=before[0], device=before[1],
                page_url=self.binding.page_url, effective_proxy=self.binding.launch[2])
        except BaseException as error:
            if isinstance(error, asyncio.CancelledError):
                self.browser_unknown = True
            self.failed = True
            self.fence()
        finally:
            self.spec = None

    def close(self, native_state, resume):
        if self.completion is not None:
            if not self.completion.done() and (native_state == 'unknown' or not resume):
                self.fence()
            return
        self.closed, self.payload = True, None
        if (native_state == 'unknown' or not resume or self.failed
                or self.browser_unknown or not self.owner.done() or self.ledger.pending):
            self.fence()
        self.completion = self.control._retain(self.complete(native_state, resume))

    async def complete(self, native_state, resume):
        state, settled = 'fenced', False
        try:
            # Closing admission precedes this wait. Raw RPCs are never cancelled.
            await asyncio.shield(self.owner)
            try:
                self.binding.check()
            except Exception:
                self.fence()
            if self.failed or self.browser_unknown:
                self.fence()
            receipt = await self.lease.settle(native_state, resume)
            settled = receipt.browser_settled and not self.browser_unknown
            state = receipt.state if settled else 'fenced'
        except BaseException:
            self.browser_unknown = True
            self.fence()
        finally:
            self.payload = None
            if self.owner.done():
                self.binding = None
        return dict(self.identity, state=state, browser_settled=settled)


RESIDENT_PIN_JS = r"""(pin, input) => {
  try {
    const field = (object, key) => Object.getOwnPropertyDescriptor(object, key)?.value;
    return !!pin && pin.realm === window && pin.doc === document
      && pin.storage === window.localStorage && location.href === input.page_url
      && pin.storage.getItem('__cuid') === input.device
      && field(window, '__fzResidentWrapper') === pin.record
      && field(window, '__fzCastleM') === pin.module
      && !!pin.record && Object.isFrozen(pin.record)
      && field(pin.record, 'realm') === pin.realm
      && field(pin.record, 'module') === pin.module
      && field(pin.record, 'url') === input.wrapper_url
      && field(pin.record, 'entry') === input.wrapper_export
      && typeof pin.callable === 'function'
      && field(pin.record, 'callable') === pin.callable
      && field(pin.module, input.wrapper_export) === pin.callable;
  } catch (_) { return false; }
}"""

RESIDENT_CAPTURE_JS = r"""input => {
  const field = (object, key) => object && Object.getOwnPropertyDescriptor(object, key)?.value;
  const record = field(window, '__fzResidentWrapper');
  return Object.freeze({realm: window, doc: document, storage: window.localStorage,
    record, module: field(window, '__fzCastleM'), callable: field(record, 'callable'),
    check: __RESIDENT_PIN_CHECK__});
}""".replace('__RESIDENT_PIN_CHECK__', RESIDENT_PIN_JS)

RESIDENT_PINNED_MINT_JS = r"""async (pin, input) => {
  if (!pin.check(pin, input)) return {ok: false, code: 'resident_context'};
  const result = await (__RESIDENT_MINT__)(input);
  return pin.check(pin, input) ? result : {ok: false, code: 'resident_context'};
}""".replace('__RESIDENT_MINT__', RESIDENT_MINT_JS)

RESIDENT_FETCH_JS = r"""async (pin, input) => {
  const out = {status: null, content_type: null, body: '', bytes: 0,
    complete: false, started: 0, finished: 0, settled: false,
    failure: null, elapsed_ms: 0};
  const controller = new AbortController(), start = performance.now();
  const timer = setTimeout(() => controller.abort(), input.timeout_ms);
  let reader;
  try {
    if (!pin.check(pin, input)) {out.failure = 'context_changed'; return out;}
    const payload = JSON.parse(input.body);
    if (!payload || Array.isArray(payload) || typeof payload !== 'object'
        || Object.hasOwn(payload, 'castleToken')) throw new Error('body');
    payload.castleToken = input.token;
    out.started = 1;
    const response = await fetch('https://api.framer.com' + input.path, {
      method: 'POST', credentials: 'include', redirect: 'error', cache: 'no-store',
      headers: {'Content-Type': 'application/json; charset=utf-8',
        'X-Requested-By': 'Framer', Authorization: 'Bearer ' + input.bearer},
      body: JSON.stringify(payload), signal: controller.signal
    });
    out.status = response.status;
    out.content_type = (response.headers.get('content-type') || '').slice(0, 256);
    if (!response.body) {out.failure = 'body'; return out;}
    reader = response.body.getReader();
    const decoder = new TextDecoder('utf-8', {fatal: true});
    while (true) {
      const chunk = await reader.read();
      if (chunk.done) {
        out.body += decoder.decode(); out.complete = true; break;
      }
      if (chunk.value.byteLength > 65536 - out.bytes) {
        out.failure = 'response_limit'; out.body = ''; return out;
      }
      out.bytes += chunk.value.byteLength;
      out.body += decoder.decode(chunk.value, {stream: true});
    }
    if (!pin.check(pin, input)) out.failure = 'context_changed';
  } catch (_) {
    out.failure = controller.signal.aborted ? 'timeout' : 'transport';
    out.body = '';
  } finally {
    // Keep the abort timer alive until body cancellation itself settles.
    controller.abort();
    try {
      if (reader) {await reader.cancel(); reader.releaseLock();}
      out.settled = true; out.finished = out.started;
    } catch (_) {out.failure = 'unconfirmed'; out.settled = false;}
    if (out.settled && !pin.check(pin, input)) out.failure = 'context_changed';
    clearTimeout(timer);
    out.elapsed_ms = Math.max(0, Math.trunc(performance.now() - start));
  }
  return out;
}"""


class _CrossoverBinding(_Binding):
    def __init__(self, control, req):
        self.provenance = req['provenance']
        super().__init__(control, req)

    def check(self):
        super().check()
        if getattr(self.evidence, 'provenance', None) != self.provenance:
            _refuse()


class _ResidentRate:
    """Use the existing host policy without importing or entering that host."""
    def __init__(self, binding, device):
        self.binding, self.device = binding, device
        host, acc = binding.control.host, binding.acc
        self.window, self.note = host._request_window, host._note_request
        policy = getattr(self.window, '__func__', self.window).__globals__
        self.gap, self.capacity = policy['MINT_GAP_S'], policy['DEV_WINDOW_CAP']
        if (type(self.gap) not in (int, float) or not math.isfinite(self.gap)
                or self.gap < 0 or type(self.capacity) is not int or self.capacity <= 0):
            _refuse()
        self.lock, self.cooling, self.requests = acc.mint_lock, acc.dev_cooling, acc.dev_reqs
        self.tokens, self.cool = acc.token_pool, host._cool_device
        self.key = None if binding.owner is acc else device

    def check(self):
        b, acc, now = self.binding, self.binding.acc, time.monotonic()
        b.check()
        if (acc.mint_lock is not self.lock or acc.dev_cooling is not self.cooling
                or acc.dev_reqs is not self.requests or self.cooling is not b.life.cooling
                or self.requests is not b.life.requests):
            _refuse()
        for key in {self.key, self.device}:
            if (self.cooling.get(key, 0) > now
                    or len(self.window(acc, key, now)) >= self.capacity):
                _refuse()


    def cool_after_fetch(self, lease):
        # Cooling belongs to the captured account history, even if its document
        # navigated. Never apply a late result to replacement owners or maps.
        b, acc = self.binding, self.binding.acc
        host, req, life = b.control.host, b.req, b.life
        try:
            if (host.accounts is not b.accounts or host.lifecycles is not b.lifecycles
                    or b.accounts.get(req['account']) is not acc
                    or b.lifecycles.get(req['account']) is not life
                    or host.generation != req['host_generation']
                    or host.resident_application_generation != req['application_generation']
                    or host._closed or host._observed is not None
                    or acc.lifecycle is not life or life.incarnation != req['account_incarnation']
                    or life.activity is not b.ledger or b.ledger.current is not lease
                    or life.phase != 'resident' or life.close_task is not None or acc.closing
                    or life.resident_uncertain is not None
                    or life.active_commands != 0 or acc.active_requests != 0
                    or life.browser is not b.browser or acc.browser is not b.browser
                    or acc.resident_launch is not b.launch or acc.session_cookie != b.session
                    or acc.mint_lock is not self.lock or acc.token_pool is not self.tokens
                    or acc.dev_cooling is not self.cooling or life.cooling is not self.cooling
                    or acc.dev_reqs is not self.requests or life.requests is not self.requests
                    or acc.project != req['project'] or b.owner.page is not b.page
                    or b.owner.ctx is not b.context or b.page.context is not b.context
                    or b.context.browser is not b.browser or b.page.is_closed()
                    or not any(p is b.page for p in b.context.pages)
                    or not any(c is b.context for c in b.browser.contexts)
                    or not b.browser.is_connected()
                    or (b.owner is not acc and (b.owner.id != self.device
                        or not any(d is b.owner for d in acc.dev_objs.values())))):
                return
        except Exception:
            return
        self.cool(acc, self.key, req['account'])


class _CrossoverOperation(_Operation):
    def __init__(self, control, req, binding, lease):
        super().__init__(control, req, binding, lease)
        self.phase, self.pin, self.context = 'new', None, None
        self.baseline_digest = self.rate = None

    async def phase_call(self, req):
        action = req['action']
        expected = {'acquire': 'new', 'baseline': 'acquired',
                    'mint_native': 'baseline_passed'}
        if (self.phase != expected.get(action) or self.closed or self.failed
                or (self.owner is not None and not self.owner.done())):
            _refuse()
        self.admitted()
        self.phase = action + '_pending'
        self.owner = self.control._retain(self.run_phase(action, req))
        try:
            remaining = self.deadline - asyncio.get_running_loop().time()
            await asyncio.wait_for(asyncio.shield(self.owner), max(0, remaining))
            if self.closed or self.payload is None:
                _refuse()
            if action != 'baseline':
                self.admitted()
            payload, self.payload = self.payload, None
            self.phase = ('baseline_passed' if payload['accepted'] else 'rejected') \
                if action == 'baseline' else payload['state']
            return payload
        except BaseException:
            self.close('unknown', False)
            raise

    def arguments(self):
        return dict(wrapper_url=self.spec['wrapper_url'],
            wrapper_export=self.spec['wrapper_export'],
            page_url=self.binding.page_url, device=self.context[1])

    def context_payload(self, state, **fields):
        return dict(self.identity, state=state, browser_settled=not self.browser_unknown,
            cookie_header=self.context[0], device=self.context[1],
            page_url=self.binding.page_url, effective_proxy=self.binding.launch[2],
            provenance=self.binding.provenance, **fields)

    async def check_pin(self):
        if not _same_cookie_snapshot(await self.snapshot(), self.context):
            _refuse()
        if await self.rpc(self.pin.evaluate, RESIDENT_PIN_JS, self.arguments()) is not True:
            _refuse()
        if not _same_cookie_snapshot(await self.snapshot(), self.context):
            _refuse()

    async def capture(self):
        self.context = await self.snapshot()
        # Publish the handle inside the raw RPC owner, even if its waiter dies.
        async def capture_owned():
            self.pin = await self.binding.page.evaluate_handle(
                RESIDENT_CAPTURE_JS, self.arguments())
        await self.rpc(capture_owned)
        await self.check_pin()
        self.payload = self.context_payload('acquired')

    async def run_phase(self, action, req):
        try:
            if action == 'acquire':
                await self.capture()
            elif action == 'baseline':
                await self.baseline(req)
            else:
                await self.mint_native()
        except BaseException as error:
            if isinstance(error, asyncio.CancelledError):
                self.browser_unknown = True
            self.failed = True
            self.fence()
            if action == 'baseline':
                if self.payload is None:
                    self.payload = self.context_payload('baseline_complete',
                        status=None, body_complete=False, json_object=False,
                        accepted=False, refusal=None)
                self.payload.update(accepted=False, browser_settled=not self.browser_unknown)

    async def charged_mint(self):
        await self.check_pin()
        if self.rate is None:
            self.rate = _ResidentRate(self.binding, self.context[1])
        rate, acc = self.rate, self.binding.acc
        async with rate.lock:
            rate.check()
            if (type(acc.last_mint) not in (int, float)
                    or not math.isfinite(acc.last_mint) or acc.last_mint < 0):
                _refuse()
            gap = rate.gap - (time.monotonic() - acc.last_mint)
            if gap > 0:
                if gap >= self.deadline - asyncio.get_running_loop().time():
                    _refuse()
                await asyncio.sleep(gap)
            await self.check_pin()
            rate.check()
            self.admitted()
            # Conservatively charge each attempted genuine mint, even on failure.
            rate.note(acc, rate.key)
            acc.last_mint = time.monotonic()
            try:
                result = await self.rpc(self.pin.evaluate, RESIDENT_PINNED_MINT_JS,
                                        self.arguments())
            finally:
                acc.last_mint = time.monotonic()
            if (type(result) is not dict or set(result) != {'ok', 'token'}
                    or result['ok'] is not True or not _text(result['token'], LIMIT)):
                _refuse()
            await self.check_pin()
            return result['token']

    def record_baseline(self, raw, path):
        evidence = baseline_result(dict(handle=self.identity['lease'],
            project=self.spec['project'], path=path), raw)
        if not evidence['fetch_settled']:
            self.browser_unknown = True
            self.fence()
        json_object = False
        if evidence['response_complete']:
            try:
                json_object = type(comparison_json(raw['body'])) is dict
            except (ValueError, TypeError, RecursionError):
                pass
        self.payload = self.context_payload('baseline_complete', status=evidence['status'],
            body_complete=evidence['response_complete'], json_object=json_object,
            accepted=False, refusal=evidence['refusal'])
        return evidence

    async def baseline(self, req):
        token = await self.charged_mint()
        self.baseline_digest = hashlib.sha256(token.encode()).digest()
        args = dict(self.arguments(), path=self.spec['path'], bearer=req['bearer'],
            body=json.dumps(req['body'], ensure_ascii=False, allow_nan=False,
                            separators=(',', ':')), token=token,
            timeout_ms=max(1, min(30_000, int(1000 * (
                self.deadline - asyncio.get_running_loop().time())))))
        aliases = {'/ai/v3/openai/responses/': '/ai/v3/responses/',
                   '/ai/v3/anthropic/messages/': '/ai/v3/messages/'}
        path = aliases.get(self.spec['path'], self.spec['path'])
        raw = _FAILED
        async def fetch_owned():
            nonlocal raw
            raw = await self.pin.evaluate(RESIDENT_FETCH_JS, args)
            return raw
        try:
            await self.rpc(fetch_owned)
        finally:
            # Preserve observed facts even if rpc's post-check refuses a changed
            # document. Only the unchanged admission path below can accept them.
            if raw is not _FAILED:
                evidence = self.record_baseline(raw, path)
                if evidence['status'] == 403 and raw.get('elapsed_ms', 4000) < 4000:
                    self.rate.cool_after_fetch(self.lease)
        await self.check_pin()
        self.payload['accepted'] = evidence['accepted'] and path in (
            '/ai/v3/responses/', '/ai/v3/messages/')
        if not self.payload['accepted']:
            self.failed = True

    async def mint_native(self):
        token = await self.charged_mint()
        if hashlib.sha256(token.encode()).digest() == self.baseline_digest:
            _refuse()
        self.payload = self.context_payload('minted', token=token)

    async def complete(self, native_state, resume):
        try:
            await asyncio.shield(self.owner)
            await asyncio.gather(*(asyncio.shield(task)
                for task in tuple(self.ledger.pending)), return_exceptions=True)
            if self.pin is not None:
                # Disposal releases only our JS handle, never a page or browser.
                # It is retained and joined even when the caller is cancelled.
                await asyncio.shield(self.ledger.track(self.pin.dispose))
                self.pin = None
        except BaseException:
            self.browser_unknown = True
            self.fence()
        receipt = await super().complete(native_state, resume)
        self.spec = self.context = self.baseline_digest = self.rate = None
        return receipt
