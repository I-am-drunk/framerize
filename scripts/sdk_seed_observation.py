"""Private ordinary-V2 observations around existing calls; no persistence or logs."""
import asyncio
import hashlib
import math
import re
import uuid

from contextual_authorization import login_cookie


def _uuid(value):
    if type(value) is not str:
        return False
    try:
        parsed = uuid.UUID(value)
        return parsed.int != 0 and str(parsed) == value
    except ValueError:
        return False


def _session_digest(header):
    try:
        return hashlib.sha256(login_cookie(header).encode()).digest()
    except (ValueError, TypeError, UnicodeError):
        return None


def _user(value):
    return (type(value) is str and 0 < len(value) <= 128
            and value.strip() == value and all(32 <= ord(c) < 127 for c in value))


def _seed(value):
    return (type(value) is str and re.fullmatch(
        r'[0-9a-f]{12}[0-5][0-9a-f]{3}[89ab][0-9a-f]{15}', value) is not None)


class _Session:
    def __init__(self, host, account, life, user, digest):
        self.account, self.generation = account, host.generation
        self.incarnation, self.life = life.incarnation, life
        self.user, self.digest = user, digest
        self.identity = str(uuid.uuid4())
        self.contexts = []
        self.observation = None
        self.revoked = False

    def matches(self, host, account, life):
        return (not self.revoked and account == self.account and life is self.life
                and host.generation == self.generation
                and life.incarnation == self.incarnation
                and not getattr(host, '_closed', False))


def _session(host, account):
    life = host.lifecycles.get(account)
    session = getattr(life, '_sdk_session', None)
    if type(session) is _Session and session.matches(host, account, life):
        return session
    return None


def invalidate(host, account):
    life = host.lifecycles.get(account)
    session = getattr(life, '_sdk_session', None)
    if type(session) is _Session:
        session.revoked = True
        session.contexts.clear()
        if session.observation is not None:
            session.observation.invalidate()


def begin_session_change(host, account):
    invalidate(host, account)
    life = host.lifecycles.get(account)
    if life is None or not getattr(host, '_sdk_seed_enabled', False):
        return None
    life._sdk_session = None
    epoch = object()
    life._sdk_session_epoch = epoch
    return epoch


def record_verified_session(host, account, acc, epoch, user, header):
    life, digest = host.lifecycles.get(account), _session_digest(header)
    if (epoch is None or getattr(life, '_sdk_session_epoch', None) is not epoch
            or not _user(user) or digest is None or not _uuid(account)):
        return
    if (host.accounts.get(account) is not acc or acc.lifecycle is not life
            or life.phase != 'resident' or acc.closing
            or type(life.incarnation) is not int or not 0 < life.incarnation < 2**64
            or getattr(host, '_closed', False)):
        return
    session = _Session(host, account, life, user, digest)
    session.contexts.append(acc.ctx)
    life._sdk_session = session


def cookie_change(host, account, value):
    session = _session(host, account)
    if (session is None or type(value) is not str
            or session.digest != _session_digest('session=' + value)):
        begin_session_change(host, account)
        return None
    session.contexts.clear()
    if session.observation is not None:
        session.observation.invalidate()
    return session


def context_seeded(host, account, session, context):
    if session is not None and _session(host, account) is session:
        if not any(context is item for item in session.contexts):
            session.contexts.append(context)


def session_for_cookie(host, account, cookie):
    session = _session(host, account)
    if (session is not None and type(cookie) is tuple and len(cookie) == 2
            and cookie[0] == 'session' and type(cookie[1]) is str
            and session.digest == _session_digest('session=' + cookie[1])):
        return session
    return None


def _drop_observation(session):
    if session is not None and session.observation is not None:
        session.observation.invalidate()


def invalidate_observation(host, account):
    _drop_observation(_session(host, account))


class _Observation:
    def __init__(self, host, account, acc, owner, session):
        self.host, self.account, self.acc = host, account, acc
        self.owner, self.session = owner, session
        self.page, self.context, self.browser = owner.page, owner.ctx, acc.browser
        self.frame = self.page.main_frame
        self.http_device = owner.id
        self.identity = str(uuid.uuid4())
        self.revision = host.proxy_revisions.get(account, 0)
        self.proxy = host.proxies.get(account)
        self.launch_route = getattr(session.life, '_sdk_launch_route', None)
        self.rate_cooling = session.life.cooling
        self.rate_requests = session.life.requests
        self.rate_last_mint = session.life.last_mint
        self.seed = None
        self.invalid = self.validated = self.joined = self.transferred = False
        self.listeners = []

    def bound(self):
        launch = self.launch_route
        return (not self.invalid and _session(self.host, self.account) is self.session
                and self.session.observation is self
                and type(launch) is tuple and len(launch) == 2
                and type(launch[0]) is int and launch[0] == self.session.incarnation
                and (launch[1] is None or type(launch[1]) is str)
                and launch[1] == self.proxy
                and getattr(self.session.life, '_sdk_launch_route', None) is launch
                and self.host.proxy_revisions.get(self.account, 0) == self.revision
                and self.host.proxies.get(self.account) == self.proxy)

    def live(self):
        try:
            life = self.session.life
            return (self.bound() and self.host.accounts.get(self.account) is self.acc
                and self.acc.lifecycle is life and life.browser is self.browser
                and getattr(life, '_sdk_launch_browser', None) is self.browser
                and self.acc.browser is self.browser and self.owner.page is self.page
                and self.acc.primary_dev == self.http_device and self.owner.id == self.http_device
                and self.owner.ctx is self.context and self.page.context is self.context
                and self.page.main_frame is self.frame and not self.page.is_closed()
                and self.browser.is_connected() and self.context.browser is self.browser
                and any(self.page is page for page in self.context.pages)
                and any(self.context is context for context in self.browser.contexts)
                and any(self.owner is d for d in self.acc.dev_objs.values())
                and any(self.context is c for c in self.session.contexts))
        except Exception:
            return False

    def detach(self):
        listeners, self.listeners = self.listeners, []
        for source, event, callback in listeners:
            try:
                source.remove_listener(event, callback)
            except Exception:
                pass

    def invalidate(self):
        self.invalid, self.seed = True, None
        if self.session.observation is self:
            self.session.observation = None
        self.detach()
        self.acc = self.owner = self.page = self.context = self.browser = self.frame = None

    def arguments(self):
        return dict(observation_id=self.identity, session_id=self.session.identity,
                    account=self.account, generation=self.session.generation,
                    account_incarnation=str(self.session.incarnation))

    def watch(self):
        def navigated(frame):
            if frame is self.frame:
                self.invalidate()
        def invalidated(*_):
            self.invalidate()
        try:
            for source, event, callback in (
                    (self.page, 'framenavigated', navigated),
                    (self.page, 'close', invalidated),
                    (self.context, 'close', invalidated),
                    (self.browser, 'disconnected', invalidated)):
                self.listeners.append((source, event, callback))
                source.on(event, callback)
            return self.live()
        except Exception:
            self.invalidate()
            return False


def begin_observation(host, account, page):
    session = _session(host, account)
    acc = host.accounts.get(account)
    if session is None or acc is None or acc.closing or session.life.phase != 'resident':
        return None
    owner = next((d for d in acc.dev_objs.values() if d.page is page), None)
    if owner is None or owner.id != acc.primary_dev:
        return None
    if session_for_cookie(host, account, acc.session_cookie) is not session:
        return None
    _drop_observation(session)
    record = _Observation(host, account, acc, owner, session)
    session.observation = record
    if not record.watch():
        record.invalidate()
        return None
    return record


OBSERVE_JS = r"""async input => {
  const field = (object, key) => object && Object.getOwnPropertyDescriptor(object, key)?.value;
  let capture = null, selectedToken = null;
  const observed = async (module, url, call) => {
    capture = null;
    let before, observe;
    try {
      before = {realm: window, doc: document, storage: window.localStorage,
        module, callable: field(module, 'a'), url, page_url: location.href};
      observe = await (__OBSERVER__)({wrapper_url: url, wrapper_export: 'a'});
    } catch {}
    const token = await call();
    try {
      const sdk = observe?.(), seed = before.storage.getItem('__cuid');
      if (typeof token === 'string' && token.length > 500 && token.length <= 16384
          && sdk?.complete === true && sdk.v2 === 1 && sdk.v3 === 0 && sdk.missing === 0
          && before.realm === window && before.doc === document
          && before.storage === window.localStorage && before.page_url === location.href
          && before.callable === field(module, 'a') && typeof before.callable === 'function'
          && typeof seed === 'string' && /^[0-9a-f]{12}[0-5][0-9a-f]{3}[89ab][0-9a-f]{15}$/.test(seed)) {
        capture = {...before, seed};
        selectedToken = token;
      }
    } catch {}
    return token;
  };
  const value = await (__SOURCE__);
  let observation = null;
  try {
    const wrapper = field(window, '__fzResidentWrapper');
    if (capture && (value === true || value === selectedToken)
        && capture.realm === window && capture.doc === document
        && capture.storage === window.localStorage && capture.page_url === location.href
        && field(window, '__fzCastleM') === capture.module
        && wrapper && Object.isFrozen(wrapper) && field(wrapper, 'realm') === window
        && field(wrapper, 'module') === capture.module && field(wrapper, 'url') === capture.url
        && field(wrapper, 'entry') === 'a' && field(wrapper, 'callable') === capture.callable
        && field(capture.module, 'a') === capture.callable) {
      let record;
      const current = () => field(window, '__fzOrdinaryV2Observation') === record;
      record = Object.freeze({...capture, ...input, wrapper, current});
      window.__fzOrdinaryV2Observation = record;
      if (field(window, '__fzOrdinaryV2Observation') === record)
        observation = {observation_id: input.observation_id, seed: capture.seed};
    }
  } catch {}
  selectedToken = null;
  return {value, observation};
}"""


def observation_source(source):
    from resident_authorization import RESIDENT_SDK_OBSERVER_JS
    for expression, module, url in (
        ('window.__fzCastleM.a()', 'window.__fzCastleM', 'window.__fzResidentWrapper?.url'),
        ('m.a()', 'm', 'window.__fzResidentWrapper?.url'),
        ('fresh.a()', 'fresh', 'url'),
    ):
        source = source.replace('await ' + expression,
            'await observed(' + module + ', ' + url + ', () => ' + expression + ')')
    return OBSERVE_JS.replace('__OBSERVER__', RESIDENT_SDK_OBSERVER_JS).replace('__SOURCE__', source)


def observed_result(record, result):
    if type(result) is not dict or set(result) != {'value', 'observation'}:
        record.invalidate()
        raise RuntimeError('invalid SDK observation result') from None
    observation = result['observation']
    if (record.live() and type(observation) is dict
            and set(observation) == {'observation_id', 'seed'}
            and observation['observation_id'] == record.identity and _seed(observation['seed'])):
        record.seed = observation['seed']
    else:
        record.invalidate()
    return result['value']


async def close_with_observation(host, account, acc, receipt, deadline):
    """Runs inside the existing close RPC owner and its unchanged deadline."""
    session = _session(host, account)
    record = session.observation if session is not None else None
    validation = None
    if record is not None:
        record.validated = record.joined = False
    try:
        if (record is not None and record.live() and _seed(record.seed)
                and record.acc is acc and acc.primary_dev == record.http_device
                and receipt is session.life.close_receipt and receipt.browser is record.browser):
            arguments = dict(record.arguments(), seed=record.seed)
            # One bounded validation leaves the existing owner time to close.
            budget = min(1.0, max(0.0, deadline - asyncio.get_running_loop().time()))
            if budget > 0:
                validation = asyncio.create_task(record.page.evaluate(VALIDATE_JS, arguments))
                done, _ = await asyncio.wait((validation,), timeout=budget)
                if done:
                    result = validation.result()
                    record.validated = (type(result) is dict and set(result) == {'valid'}
                                        and result['valid'] is True and record.live())
    except Exception:
        if record is not None:
            record.validated = False
    finally:
        if validation is not None and not validation.done():
            validation.cancel()
        if record is not None:
            record.detach()
        try:
            await receipt.close()
        except BaseException:
            if record is not None:
                record.invalidate()
            raise
        finally:
            # Join after closing, so validation cancellation cannot block close.
            if validation is not None:
                await asyncio.gather(validation, return_exceptions=True)
        if record is not None:
            record.joined = record.validated and record.bound()
            if not record.joined:
                record.invalidate()


def transfer_closed(host, account, life, acc):
    session = _session(host, account)
    record = session.observation if session is not None else None
    if record is None:
        return
    if (not record.bound() or not record.joined or not record.validated
            or record.acc is not acc or session.life is not life
            or life.phase != 'absent' or account in host.accounts
            or life.browser is not None or life.close_receipt is not None
            or getattr(life, '_sdk_launch_browser', None) is not None
            or life.active_commands or acc.primary_dev != record.http_device):
        record.invalidate()
        return
    record.transferred = True
    record.acc = record.owner = record.page = record.context = record.browser = record.frame = None
    session.contexts.clear()


def _available(record, host, account, life, device):
    return (type(record) is _Observation and record.host is host
            and record.account == account and record.session.life is life
            and record.bound() and record.transferred and record.joined
            and record.validated and record.http_device == device and _seed(record.seed)
            and life.cooling is record.rate_cooling and life.requests is record.rate_requests
            and type(life.last_mint) in (int, float) and math.isfinite(life.last_mint)
            and type(record.rate_last_mint) in (int, float)
            and math.isfinite(record.rate_last_mint)
            and life.last_mint >= record.rate_last_mint >= 0)


def closed_rate_observation(host, account, life):
    """Authority from this host's verified session and intact, joined ledger."""
    session = _session(host, account)
    record = session.observation if session is not None else None
    device = getattr(life, 'restored_primary_device', None)
    return record if _available(record, host, account, life, device) else None


def attach(host, request, life, grant):
    keys = {'account', 'generation', 'lease', 'user', 'session_cookie'}
    if (type(request) is not dict or not keys <= request.keys()
            or request.keys() - keys - {'cmd', 'id'}
            or any(type(key) is not str for key in request)
            or not _uuid(request['account']) or not _uuid(request['lease'])):
        return None
    session = _session(host, request['account'])
    record = session.observation if session is not None else None
    if (not _available(record, host, request['account'], life, grant.device)
            or not _user(request['user']) or request['user'] != session.user
            or _session_digest(request['session_cookie']) != session.digest
            or grant.waiting or grant.minting or grant.sequence != 0
            or getattr(grant, 'sdk_observation', None) not in (None, record)):
        return None
    grant.sdk_observation = record
    return dict(account=record.account, generation=session.generation,
                lease=request['lease'], account_incarnation=session.incarnation,
                user=session.user, session_id=session.identity,
                observation_id=record.identity, seed=record.seed)


def echo(grant):
    record = getattr(grant, 'sdk_observation', None)
    return ({} if record is None else
            dict(session_id=record.session.identity, observation_id=record.identity))


def matching_request(grant, request):
    fields = echo(grant)
    return all((key not in request if key not in fields else
                type(request.get(key)) is str and request[key] == fields[key])
               for key in ('session_id', 'observation_id'))


def ready(host, account, life, grant):
    record = getattr(grant, 'sdk_observation', None)
    return (record is None or _available(record, host, account, life, grant.device))


VALIDATE_JS = r"""input => {
  try {
    const field = (object, key) => object && Object.getOwnPropertyDescriptor(object, key)?.value;
    const record = field(window, '__fzOrdinaryV2Observation');
    if (!record || !Object.isFrozen(record)) return {valid: false};
    for (const key of ['account', 'generation', 'account_incarnation', 'session_id', 'observation_id'])
      if (field(record, key) !== input[key]) return {valid: false};
    const module = field(record, 'module'), wrapper = field(record, 'wrapper');
    const storage = field(record, 'storage'), callable = field(record, 'callable');
    return {valid: field(record, 'realm') === window && field(record, 'doc') === document
      && storage === window.localStorage && field(record, 'page_url') === location.href
      && field(record, 'current')() === true
      && field(window, '__fzCastleM') === module && field(module, 'a') === callable
      && field(window, '__fzResidentWrapper') === wrapper && Object.isFrozen(wrapper)
      && field(wrapper, 'module') === module && field(wrapper, 'callable') === callable
      && field(wrapper, 'realm') === window && field(wrapper, 'entry') === 'a'
      && field(wrapper, 'url') === field(record, 'url')
      && field(record, 'seed') === input.seed && storage.getItem('__cuid') === input.seed};
  } catch { return {valid: false}; }
}"""
