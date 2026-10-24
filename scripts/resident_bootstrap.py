"""Private read-only bootstrap for an existing resident page. No SDK call."""
import asyncio
from resident_activity import ResidentRefused
from resident_authorization import (
    _text, _positive, _uuid, _valid_page_url, _validate_acquire, ROUTES, LIMIT,
    cookies_for_url, snapshot_header, login_cookie,
)

COMMAND = 'resident_authorization_bootstrap'
FIELDS = frozenset(('version', 'account', 'application_generation', 'bootstrap',
                    'project', 'path', 'proxy_revision'))

def refuse():
    raise ResidentRefused('resident bootstrap refused') from None

def validate(req):
    try:
        if (type(req) is not dict or not FIELDS <= req.keys()
                or req.keys() - FIELDS - {'id', 'cmd'}
                or type(req['version']) is not int or req['version'] != 2
                or not _uuid(req['account']) or not _uuid(req['bootstrap'])
                or not _positive(req['application_generation'])
                or not _text(req['project'], 128)
                or not all(c.isascii() and (c.isalnum() or c in '_-') for c in req['project'])
                or req['path'] not in ROUTES
                or type(req['proxy_revision']) is not int
                or not 0 <= req['proxy_revision'] < 2 ** 64
                or ('cmd' in req and req['cmd'] != COMMAND)
                or ('id' in req and (type(req['id']) is not int or not 0 <= req['id'] < 2 ** 64))):
            refuse()
        return dict(req)
    except (ValueError, TypeError, AttributeError, KeyError):
        refuse()


READ_JS = r"""() => {
  const realm = window, doc = document, store = window.localStorage;
  const field = (o, k) => Object.getOwnPropertyDescriptor(o, k)?.value;
  const record = field(window, '__fzResidentWrapper');
  const module = field(window, '__fzCastleM');
  if (!record || !module || !Object.isFrozen(record)) return {ok:false};
  const url = field(record, 'url'), entry = field(record, 'entry');
  const callable = field(record, 'callable');
  if (field(record, 'module') !== module || field(record, 'realm') !== realm
      || typeof callable !== 'function' || field(module, entry) !== callable
      || typeof url !== 'string' || typeof entry !== 'string') return {ok:false};
  const page_url = location.href, device = store.getItem('__cuid');
  if (window !== realm || document !== doc || window.localStorage !== store
      || field(window, '__fzResidentWrapper') !== record
      || field(window, '__fzCastleM') !== module) return {ok:false};
  return {ok:true, page_url, device, wrapper_url:url, wrapper_export:entry};
}"""


class Bootstrap:
    def __init__(self, host, req):
        self.host, self.req = host, req
        self.acc = host.accounts.get(req['account'])
        self.life = host.lifecycles.get(req['account'])
        self.evidence = getattr(self.acc, 'completed_browser_http200', None)
        self.unknown = False
        self.lease = None
        self.check()
        self.page_url, self.session = self.evidence.page.url, self.acc.session_cookie
        self.ledger = self.life.activity
        self.lease = self.ledger.acquire(req['bootstrap'])

    def check(self):
        host, req, acc, life, e = self.host, self.req, self.acc, self.life, self.evidence
        if (acc is None or life is None or e is None or not e.current()
                or host.accounts.get(req['account']) is not acc
                or host.lifecycles.get(req['account']) is not life):
            refuse()
        if (host._closed or host._observed is not None
                or host.resident_application_generation != req['application_generation']
                or e.host is not host or e.account != req['account'] or e.acc is not acc
                or e.project != req['project'] or not _uuid(e.provenance)
                or acc.completed_browser_http200 is not e or acc.lifecycle is not life
                or life.active_commands != 0 or acc.active_requests != 0
                or life.close_task is not None or life.resident_uncertain is not None
                or host.proxy_revisions.get(req['account'], 0) != req['proxy_revision']
                or not _positive(life.incarnation) or not _text(host.generation, 128)
                or type(e.launch) is not tuple or len(e.launch) != 4
                or e.launch[3] is not False
                or (e.launch[2] is not None and not _text(e.launch[2], LIMIT))):
            refuse()
        if self.lease is not None and (life.activity is not self.ledger
                or self.ledger.current is not self.lease or self.ledger.fenced
                or e.page.url != self.page_url or acc.session_cookie != self.session):
            refuse()

    async def rpc(self, call, *args):
        self.check()
        try:
            value = await self.lease.rpc(lambda: call(*args))
        except BaseException:
            self.unknown = True
            raise
        self.check()
        return value

    async def snapshot(self):
        url = 'https://api.framer.com' + self.req['path']
        rows = cookies_for_url(await self.rpc(self.evidence.context.cookies, url), url)
        header = snapshot_header(rows)
        devices = [row['value'] for row in rows if row.get('name') == '__cuid']
        if (len(devices) != 1 or not _text(devices[0], 256)
                or type(self.session) is not tuple or len(self.session) != 2
                or self.session[0] != 'session' or login_cookie(header) != self.session[1]
                or (self.evidence.owner is not self.acc and self.evidence.owner.id != devices[0])):
            refuse()
        return header, devices[0]

    async def run(self):
        try:
            if not _valid_page_url(self.page_url):
                refuse()
            before = await self.snapshot()
            data = await self.rpc(self.evidence.page.evaluate, READ_JS)
            if (type(data) is not dict or set(data) != {
                    'ok', 'page_url', 'device', 'wrapper_url', 'wrapper_export'}
                    or data['ok'] is not True or data['page_url'] != self.page_url
                    or data['device'] != before[1] or await self.snapshot() != before):
                refuse()
            _validate_acquire(dict(project=self.req['project'], path=self.req['path'],
                wrapper_url=data['wrapper_url'], wrapper_export=data['wrapper_export'],
                cookie_header=before[0], budget_ms=2000))
            self.check()
            result = dict(version=2, account=self.req['account'],
                application_generation=self.req['application_generation'],
                bootstrap=self.req['bootstrap'], host_generation=self.host.generation,
                account_incarnation=self.life.incarnation, project=self.req['project'],
                path=self.req['path'], proxy_revision=self.req['proxy_revision'],
                provenance=self.evidence.provenance, page_url=self.page_url,
                device=before[1], effective_proxy=self.evidence.launch[2],
                wrapper_url=data['wrapper_url'], wrapper_export=data['wrapper_export'])
        finally:
            receipt = await self.lease.settle('unknown' if self.unknown else 'not_started', True)
        if receipt.state != 'released' or not receipt.browser_settled:
            refuse()
        return dict(result, browser_settled=True)


async def command(host, request):
    req = validate(request)
    try:
        operation = Bootstrap(host, req)
        task = asyncio.create_task(operation.run())
        owners = getattr(host, '_resident_bootstrap_tasks', None)
        if owners is None:
            owners = host._resident_bootstrap_tasks = set()
        owners.add(task)
        def completed(done):
            owners.discard(done)
            if not done.cancelled():
                done.exception()
        task.add_done_callback(completed)
        return await asyncio.wait_for(asyncio.shield(task), 2)
    except Exception:
        refuse()
