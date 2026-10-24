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

The selected HTTP-200 page may be /login. Project authority comes solely
from the completed response provenance. No editor readiness is consulted.
Wrapper metadata must be a frozen data record created at the actual cache
installation: {module, callable, url, entry, realm: window}. No metadata is
inferred from an old cache. Root must gate all ordinary host activity and
retain the native operation until its settlement; this module proves neither
native settlement nor page-script quiescence beyond its observed barriers.
"""
import asyncio
import re
import uuid
from urllib.parse import urlsplit

from contextual_authorization import cookies_for_url, snapshot_header, login_cookie
from resident_activity import ActivityLedger, ResidentRefused

COMMAND = 'resident_authorization_once'
LIMIT = 16 * 1024
MAX_BUDGET_MS = 45_000
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
        if (type(request['version']) is not int or request['version'] != 1
                or not _uuid(request['account']) or not _uuid(request['lease'])
                or not _positive(request['application_generation'])
                or not _positive(request['account_incarnation'])
                or not _text(request['host_generation'], 128)):
            _refuse()
        if action == 'acquire':
            _validate_acquire(request)
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
                return await self._acquire(req)
            op = self._lookup(req)
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
