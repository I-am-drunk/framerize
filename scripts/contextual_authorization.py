"""One direct wrapper call on the actual project page, with private cookies."""
import asyncio
import json
import re
import secrets
import time
from urllib.parse import urlsplit

API = "https://api.framer.com"
ROUTES = frozenset(("/ai/v3/responses/", "/ai/v3/messages/"))
LIMIT = 16 * 1024
COMPARISON_WRAPPER = "https://app.framerstatic.com/chunk-VPFQZMNG.mjs"
COMPARISON_EXPORT = "b"
COMPARISON_BYTES = 64 * 1024
COMPARISON_SECONDS = 180
BASELINE_SECONDS = 30
COMPARISON_READY_SECONDS = 10
COMPARISON_DISPOSE_SECONDS = 2
DIAGNOSTIC_STAGES = frozenset(('account_start', 'context_inputs', 'cookie_seed',
    'navigation', 'device_readiness', 'snapshot_page', 'snapshot_cookies', 'snapshot_header',
    'snapshot_validate', 'context_ready', 'baseline', 'native_token'))
DIAGNOSTIC_CHECKS = ('device_valid', 'device_cookie_match', 'login_match',
                     'page_match', 'url_unchanged')


def comparison_mark(scope, stage, **checks):
    scope.comparison_diagnostic_stage = stage if stage in DIAGNOSTIC_STAGES else 'unknown'
    scope.comparison_diagnostic_checks = {key: checks[key] for key in DIAGNOSTIC_CHECKS
        if type(checks.get(key)) is bool}


def comparison_evidence(scope):
    stage = getattr(scope, 'comparison_diagnostic_stage', None)
    result = {'stage': stage if type(stage) is str and stage in DIAGNOSTIC_STAGES else 'unknown'}
    checks = getattr(scope, 'comparison_diagnostic_checks', None)
    if type(checks) is dict:
        result.update({key: checks[key] for key in DIAGNOSTIC_CHECKS
                       if type(checks.get(key)) is bool})
    return result


def reserve_comparison(scope, request):
    """Reserve each phase before scheduling its command; ordinary mode is separate."""
    phase = getattr(scope, "comparison_phase", None)
    pair = (request.get("cmd"), request.get("purpose"))
    expected = {None: ("prepare_inference_authorization", "baseline_context"),
                "context_ready": ("browser_baseline_once", None),
                "baseline_passed": ("prepare_inference_authorization", "native_token")}
    previous = getattr(scope, "comparison_tail", None)
    busy = scope.commands and (previous is None or phase is None or any(
        task is not previous and (task is None or not task.done())
        for task in scope.commands.values()))
    if (pair != expected.get(phase) or busy
            or (phase is None and (scope.launch_started or scope.contextual_authorization))):
        raise ValueError("invalid browser comparison sequence")
    scope.contextual_authorization = True
    scope.comparison_phase = {None: "context_pending", "context_ready": "baseline_pending",
                              "baseline_passed": "native_pending"}[phase]
    scope.comparison_next = "failed"
    return previous


def page_matches(url, project):
    if type(url) is not str or len(url.encode()) > 4096:
        return False
    try:
        parsed = urlsplit(url)
        if (parsed.scheme != "https" or parsed.hostname != "framer.com"
                or parsed.username is not None or parsed.password is not None
                or parsed.port not in (None, 443)):
            return False
        prefix = "/projects/"
        path = parsed.path.rstrip("/")
        if not path.startswith(prefix):
            return False
        last = path[len(prefix):]
        return "/" not in last and (last == project or last.endswith("--" + project))
    except ValueError:
        return False


def require_running(scope):
    if scope.failure or scope.stopped.is_set():
        raise RuntimeError("contextual authorization stopped")


async def rpc(scope, make):
    require_running(scope)
    return await asyncio.shield(scope.rpc(make()))


async def initialize_account(host, account, browser, scope, make_account):
    """No legacy warm, device pool, synthetic input or heartbeat is started."""
    context = await rpc(scope, browser.new_context)
    page = await rpc(scope, context.new_page)
    require_running(scope)
    value = make_account(None, context, page)
    value._observed_scope = scope
    value.browser = browser
    host.accounts[account] = value
    return value


def login_cookie(header):
    if (type(header) is not str or not header or len(header.encode()) > LIMIT
            or any(ord(c) < 32 or ord(c) > 126 for c in header)):
        raise ValueError("invalid private cookie snapshot")
    login = None
    allowed = "!#$%&'*+-.^_|~" + chr(96)
    for part in header.split(";"):
        name, separator, value = part.strip().partition("=")
        if (not separator or not name
                or any(not c.isalnum() and c not in allowed for c in name)
                or any(ord(c) < 33 or ord(c) in (34, 44, 92) for c in value)):
            raise ValueError("invalid private cookie snapshot")
        if name == "session":
            if login is not None or not value:
                raise ValueError("ambiguous private login")
            login = value
    if login is None:
        raise ValueError("missing private login")
    return login


def validate_request(request):
    project, path = request.get("project"), request.get("path")
    if (type(project) is not str or not re.fullmatch(r"[A-Za-z0-9_-]{1,256}", project)
            or type(path) is not str or path not in ROUTES):
        raise ValueError("invalid contextual authorization target")
    return project, path, login_cookie(request.get("cookie_header"))


DIRECT_SDK = """async (sdk) => {
  const before = location.href;
  const module = await import(sdk);
  if (typeof module.a !== "function") throw new Error("SDK export unavailable");
  const token = await module.a();
  return {before, after: location.href, token};
}"""


def snapshot_header(rows):
    if type(rows) is not list or len(rows) > 256:
        raise ValueError("invalid cookie snapshot")
    parts, size = [], 0
    allowed = "!#$%&'*+-.^_|~" + chr(96)
    for row in rows:
        if type(row) is not dict:
            raise ValueError("invalid cookie snapshot")
        name, value = row.get("name"), row.get("value")
        if (type(name) is not str or type(value) is not str or not name
                or any(ord(c) > 126 or (not c.isalnum() and c not in allowed) for c in name)
                or any(ord(c) < 33 or ord(c) > 126 or ord(c) in (34, 44, 59, 92) for c in value)):
            raise ValueError("invalid cookie snapshot")
        part = name + "=" + value
        size += len(part) + (2 if parts else 0)
        if size > LIMIT:
            raise ValueError("cookie snapshot exceeds bound")
        parts.append(part)
    header = "; ".join(parts)
    login_cookie(header)
    return header


async def prepare(host, account, sdk_url, plan):
    try:
        return await _prepare(host, account, sdk_url, plan)
    except Exception:
        raise RuntimeError("contextual authorization failed") from None


async def _prepare(host, account, sdk_url, plan):
    project, path, login = plan
    scope = host._observed.accounts[account]
    acc = host.accounts[account]
    if not scope.contextual_authorization or acc._observed_scope is not scope:
        raise RuntimeError("contextual authorization owner mismatch")
    page, context = acc.page, acc.ctx
    await rpc(scope, lambda: context.add_cookies([{
        "name": "session", "value": login, "domain": ".framer.com",
        "path": "/", "secure": True, "httpOnly": True,
    }]))
    await rpc(scope, lambda: page.goto("https://framer.com/projects/" + project,
        wait_until="domcontentloaded", timeout=45000))
    if not page_matches(page.url, project):
        raise RuntimeError("project page unavailable")
    result = await rpc(scope, lambda: page.evaluate(DIRECT_SDK, sdk_url))
    if (type(result) is not dict or not page_matches(result.get("before"), project)
            or not page_matches(result.get("after"), project)):
        raise RuntimeError("SDK page context changed")
    token = result.get("token")
    if type(token) is not str or not token or len(token.encode()) > LIMIT:
        raise RuntimeError("SDK token unavailable")
    rows = await rpc(scope, lambda: context.cookies(API + path))
    header = snapshot_header(rows)
    if login_cookie(header) != login or not page_matches(page.url, project):
        raise RuntimeError("private session context changed")
    require_running(scope)
    return {"project": project, "path": path, "page_url": page.url,
            "source": "framer_wrapper_direct", "token": token, "cookie_header": header}


def comparison_body(value, project):
    if (type(value) is not dict or value.get("projectId") != project
            or "castleToken" in value or type(value.get("request")) is not dict
            or value["request"].get("stream") is not False):
        raise ValueError("invalid comparison body")
    turn = value.get("turnId")
    if (type(turn) is not str or not turn or len(turn.encode()) > 256
            or any(ord(char) < 32 or ord(char) == 127 for char in turn)):
        raise ValueError("invalid comparison turn")
    body = json.dumps(value, ensure_ascii=False, allow_nan=False,
                      sort_keys=True, separators=(",", ":"))
    if len(body.encode()) > COMPARISON_BYTES:
        raise ValueError("comparison body exceeds limit")
    return body


def comparison_bodies(request, project):
    baseline = comparison_body(request.get("body"), project)
    native = comparison_body(request.get("native_body"), project)
    left, right = json.loads(baseline), json.loads(native)
    same_turn = left.pop("turnId") == right.pop("turnId")
    if same_turn or json.dumps(left, sort_keys=True) != json.dumps(right, sort_keys=True):
        raise ValueError("comparison envelopes do not match")
    return baseline, native


COMPARISON_READY = """() => {
  const device = localStorage.getItem('__cuid');
  return typeof device === 'string' && device.length > 0;
}"""

COMPARISON_SNAPSHOT = """() => ({
  url: location.href, device: localStorage.getItem('__cuid')
})"""

COMPARISON_MINT = """async ({url, entry, device}) => {
  const before = location.href;
  if (localStorage.getItem('__cuid') !== device) throw new Error('context');
  const module = await import(url);
  if (typeof module[entry] !== 'function') throw new Error('wrapper');
  const token = await module[entry]();
  if (localStorage.getItem('__cuid') !== device) throw new Error('context');
  return {before, after: location.href, token};
}"""


async def comparison_rpc(scope, make, timeout=45):
    value = await asyncio.wait_for(rpc(scope, make), timeout)
    require_running(scope)
    return value


def comparison_owner(host, account, scope):
    acc = host.accounts[account]
    if (host._observed.accounts.get(account) is not scope
            or acc._observed_scope is not scope):
        raise ValueError("comparison owner changed")
    return acc


async def comparison_device_ready(scope, page):
    async def wait_and_dispose():
        handle = await page.wait_for_function(COMPARISON_READY,
            timeout=COMPARISON_READY_SECONDS * 1000, polling=100)
        await asyncio.wait_for(asyncio.shield(scope.rpc(handle.dispose())),
                               COMPARISON_DISPOSE_SECONDS)

    # Both RPCs stay owned after caller cancellation or a disposal timeout.
    await comparison_rpc(scope, wait_and_dispose,
                         COMPARISON_READY_SECONDS + COMPARISON_DISPOSE_SECONDS)


async def comparison_snapshot(scope, acc, project, path, login):
    comparison_mark(scope, 'snapshot_page')
    snap = await comparison_rpc(scope, lambda: acc.page.evaluate(COMPARISON_SNAPSHOT))
    comparison_mark(scope, 'snapshot_cookies')
    rows = await comparison_rpc(scope, lambda: acc.ctx.cookies(API + path))
    comparison_mark(scope, 'snapshot_header')
    header = snapshot_header(rows)
    device = snap.get("device") if type(snap) is dict else None
    devices = [row["value"] for row in rows if row.get("name") == "__cuid"]
    comparison_mark(scope, 'snapshot_validate',
        device_valid=type(device) is str and bool(device) and len(device.encode()) <= 256,
        device_cookie_match=devices == [device], login_match=login_cookie(header) == login,
        page_match=page_matches(snap.get("url") if type(snap) is dict else None, project),
        url_unchanged=type(snap) is dict and snap.get("url") == acc.page.url)
    if (type(device) is not str or not device or len(device.encode()) > 256
            or devices != [device] or login_cookie(header) != login
            or not page_matches(snap.get("url"), project)
            or snap["url"] != acc.page.url):
        raise ValueError("comparison context unavailable")
    return snap["url"], device, header


def comparison_target(request):
    project, path = request.get("project"), request.get("path")
    if (type(project) is not str or not re.fullmatch(r"[A-Za-z0-9_-]{1,256}", project)
            or type(path) is not str or path not in ROUTES):
        raise ValueError("invalid comparison target")
    return project, path


def comparison_reply(state, purpose, header):
    return {"source": "framer_browser_comparison_v1", "purpose": purpose,
            "context_handle": state["handle"], "project": state["project"],
            "path": state["path"], "page_url": state["url"], "cookie_header": header}


def validate_comparison_request(request):
    project, _ = comparison_target(request)
    comparison_body(request.get("body"), project)
    if request.get("purpose") == "baseline_context":
        comparison_bodies(request, project)
        login_cookie(request.get("cookie_header"))
        if (request.get("wrapper_url") != COMPARISON_WRAPPER
                or request.get("wrapper_export") != COMPARISON_EXPORT):
            raise ValueError("unsupported comparison wrapper")
    else:
        handle = request.get("context_handle")
        if type(handle) is not str or not re.fullmatch(r"[A-Za-z0-9_-]{43}", handle):
            raise ValueError("invalid comparison handle")
        if request.get("cmd") == "browser_baseline_once":
            bearer = request.get("bearer")
            if (type(bearer) is not str or not bearer or len(bearer.encode()) > LIMIT
                    or any(ord(c) < 33 or ord(c) > 126 for c in bearer)):
                raise ValueError("invalid comparison bearer")
            login_cookie(request.get("cookie_header"))


async def comparison_context(host, account, scope, request):
    comparison_mark(scope, 'context_inputs')
    acc = comparison_owner(host, account, scope)
    project, path = comparison_target(request)
    baseline, native = comparison_bodies(request, project)
    login = login_cookie(request["cookie_header"])
    comparison_mark(scope, 'cookie_seed')
    await comparison_rpc(scope, lambda: acc.ctx.add_cookies([{
        "name": "session", "value": login, "domain": ".framer.com",
        "path": "/", "secure": True, "httpOnly": True}]))
    comparison_mark(scope, 'navigation')
    await comparison_rpc(scope, lambda: acc.page.goto(
        "https://framer.com/projects/" + project,
        wait_until="domcontentloaded", timeout=45000), 46)
    comparison_mark(scope, 'device_readiness')
    await comparison_device_ready(scope, acc.page)
    url, device, header = await comparison_snapshot(scope, acc, project, path, login)
    state = {"identity": scope.identity(), "account": account, "acc": acc,
             "page": acc.page, "context": acc.ctx, "browser": acc.browser,
             "project": project, "path": path, "url": url, "device": device,
             "login": login, "handle": secrets.token_urlsafe(32), "drift": False,
             "baseline": baseline, "native": native,
             "wrapper_url": request["wrapper_url"], "wrapper_export": request["wrapper_export"],
             "deadline": time.monotonic() + COMPARISON_SECONDS}

    def navigated(frame):
        if frame == acc.page.main_frame:
            state["drift"] = True

    acc.page.on("framenavigated", navigated)
    scope.comparison = state
    scope.comparison_next = "context_ready"
    comparison_mark(scope, 'context_ready')
    return comparison_reply(state, "baseline_context", header)


def comparison_identity(state, account, scope, acc):
    return (state is not None and state["identity"] == scope.identity()
            and state["account"] == account and state["acc"] is acc
            and state["page"] is acc.page and state["context"] is acc.ctx
            and state["browser"] is acc.browser and not state["drift"]
            and time.monotonic() < state["deadline"])


async def comparison_bound(host, account, scope, request, leg):
    state = scope.comparison
    acc = comparison_owner(host, account, scope)
    if (not comparison_identity(state, account, scope, acc)
            or request.get("context_handle") != state["handle"]
            or comparison_target(request) != (state["project"], state["path"])
            or comparison_body(request.get("body"), state["project"]) != state[leg]):
        raise ValueError("comparison binding changed")
    url, device, header = await comparison_snapshot(
        scope, acc, state["project"], state["path"], state["login"])
    if (not comparison_identity(state, account, scope, comparison_owner(host, account, scope))
            or url != state["url"] or device != state["device"]):
        raise ValueError("comparison device or document changed")
    return state, acc, header


async def comparison_mint(scope, state, acc):
    result = await comparison_rpc(scope, lambda: acc.page.evaluate(COMPARISON_MINT, {
        "url": state["wrapper_url"], "entry": state["wrapper_export"],
        "device": state["device"]}))
    if (type(result) is not dict or result.get("before") != state["url"]
            or result.get("after") != state["url"] or type(result.get("token")) is not str
            or not result["token"] or len(result["token"].encode()) > LIMIT):
        raise ValueError("comparison mint unavailable")
    return result["token"]


async def comparison_native(host, account, scope, request):
    comparison_mark(scope, 'native_token')
    state, acc, _ = await comparison_bound(host, account, scope, request, "native")
    token = await comparison_mint(scope, state, acc)
    _, _, header = await comparison_bound(host, account, scope, request, "native")
    reply = comparison_reply(state, "native_token", header)
    reply.update(source="framer_wrapper_direct", token=token)
    scope.comparison_next = "complete"
    return reply


COMPARISON_FETCH = """async (input) => {
  const result = {status: null, content_type: null, body: '', bytes: 0,
    complete: false, started: 0, finished: 0, settled: false,
    failure: null, elapsed_ms: 0};
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), input.timeout_ms);
  const start = performance.now();
  let reader;
  const sameContext = () => {
    const devices = document.cookie.split(';').map(p => p.trim())
      .filter(p => p.startsWith('__cuid=')).map(p => p.slice(7));
    return location.href === input.page_url &&
      localStorage.getItem('__cuid') === input.device &&
      devices.length === 1 && devices[0] === input.device;
  };
  try {
    if (!sameContext()) { result.failure = 'context_changed'; return result; }
    const payload = JSON.parse(input.body);
    payload.castleToken = input.token;
    result.started = 1;
    const response = await fetch('https://api.framer.com' + input.path, {
      method: 'POST', credentials: 'same-origin', redirect: 'error', cache: 'no-store',
      headers: {'content-type': 'application/json; charset=utf-8',
        'x-requested-by': 'Framer', authorization: 'Bearer ' + input.bearer},
      body: JSON.stringify(payload), signal: controller.signal
    });
    result.status = response.status;
    result.content_type = (response.headers.get('content-type') || '').slice(0, 256);
    if (!response.body) { result.failure = 'body'; return result; }
    reader = response.body.getReader();
    const decoder = new TextDecoder('utf-8', {fatal: true});
    while (true) {
      const chunk = await reader.read();
      if (chunk.done) {
        result.body += decoder.decode();
        result.complete = true;
        break;
      }
      if (chunk.value.byteLength > input.byte_limit - result.bytes) {
        result.failure = 'response_limit'; result.body = ''; return result;
      }
      result.bytes += chunk.value.byteLength;
      result.body += decoder.decode(chunk.value, {stream: true});
    }
    if (!sameContext()) result.failure = 'context_changed';
  } catch (_) {
    result.failure = controller.signal.aborted ? 'timeout' : 'transport';
    result.body = '';
  } finally {
    clearTimeout(timer);
    controller.abort();
    try { if (reader) { await reader.cancel(); reader.releaseLock(); }
      result.settled = true; result.finished = result.started;
    } catch (_) { result.failure = 'unconfirmed'; result.settled = false; }
    result.elapsed_ms = Math.max(0, Math.trunc(performance.now() - start));
  }
  return result;
}"""


def comparison_json(text):
    def unique(pairs):
        value = {}
        for key, item in pairs:
            if key in value:
                raise ValueError("duplicate JSON member")
            value[key] = item
        return value

    def constant(_):
        raise ValueError("invalid JSON constant")

    return json.loads(text, object_pairs_hook=unique, parse_constant=constant)


def comparison_complete(value, path):
    if (type(value) is not dict or type(value.get("id")) is not str
            or not value["id"] or value.get("error") is not None):
        return False, False
    if path == "/ai/v3/responses/":
        items = value.get("output")
        complete = value.get("status") == "completed" and value.get("incomplete_details") is None
        blocks = [block for item in items if type(item) is dict
                  and item.get("type") == "message" and type(item.get("content")) is list
                  for block in item["content"]] if type(items) is list else []
        kind = "output_text"
    else:
        items = blocks = value.get("content")
        complete = value.get("stop_reason") in ("end_turn", "stop_sequence", "tool_use")
        kind = "text"
    text = any(type(block) is dict and block.get("type") == kind
               and type(block.get("text")) is str and block["text"].strip()
               for block in blocks) if type(blocks) is list else False
    return bool(complete and type(items) is list and items), bool(text)


def comparison_refusal(text, value, elapsed_ms):
    lower = text.translate(str.maketrans("ABCDEFGHIJKLMNOPQRSTUVWXYZ", "abcdefghijklmnopqrstuvwxyz"))
    provider = ('"request_id"', "gen-", "provider", "moderation", "no endpoints found",
                "no allowed providers", "error_type", "flagged")
    if any(marker in lower for marker in provider):
        return {"policy": "upstream", "basis": "provider_marker"}
    error = value.get("error") if type(value) is dict else None
    if type(error) is dict:
        data = error.get("data")
        if (error.get("status") == 403
                and error.get("message") == "This request was blocked by our security checks"
                and type(data) is dict and data.get("reason") == "request_blocked"
                and type(error.get("ref")) is str and error["ref"]
                and type(error.get("timestamp")) is str and error["timestamp"]):
            return {"policy": "castle", "basis": "framer_security_envelope"}
    if any(marker in lower for marker in ("forbidden", "castle", "risk", "challenge", "captcha")):
        return {"policy": "castle", "basis": "framer_marker"}
    return {"policy": "upstream" if elapsed_ms >= 1000 else "castle",
            "basis": "elapsed_fallback"}


def baseline_evidence(state):
    return {"source": "framer_browser_comparison_v1", "context_handle": state["handle"],
            "project": state["project"], "path": state["path"], "accepted": False,
            "status": None, "content_type": "missing", "response_complete": False,
            "complete_json": False, "nonempty_text": False, "body_bytes": 0,
            "transport_starts": 0, "transport_finishes": 0, "fetch_settled": True,
            "failure": None, "refusal": None}


def baseline_result(state, raw):
    out = baseline_evidence(state)
    out.update(transport_starts=None, transport_finishes=None, fetch_settled=False,
               failure="protocol")
    if type(raw) is not dict:
        return out
    status, body = raw.get("status"), raw.get("body")
    if type(status) is int and 100 <= status <= 599:
        out["status"] = status
    if (type(body) is not str or len(body.encode()) > COMPARISON_BYTES
            or type(raw.get("bytes")) is not int or not 0 <= raw["bytes"] <= COMPARISON_BYTES
            or type(raw.get("started")) is not int or raw["started"] not in (0, 1)
            or type(raw.get("finished")) is not int or not 0 <= raw["finished"] <= raw["started"]
            or type(raw.get("complete")) is not bool or type(raw.get("settled")) is not bool
            or type(raw.get("elapsed_ms")) is not int or not 0 <= raw["elapsed_ms"] < 2**53):
        return out
    if ((raw["complete"] and (raw["bytes"] != len(body.encode()) or raw["started"] != 1))
            or (raw["settled"] and raw["finished"] != raw["started"])
            or (raw["finished"] == 1 and not raw["settled"])):
        return out
    media = raw.get("content_type")
    if media is not None and (type(media) is not str or len(media) > 256):
        return out
    media = (media or "").split(";", 1)[0].strip().lower()
    content_type = ("json" if media == "application/json" or
                    (media.startswith("application/") and media.endswith("+json")) else
                    "event_stream" if media == "text/event-stream" else "other" if media else "missing")
    failure = raw.get("failure")
    if failure not in (None, "context_changed", "response_limit", "body", "timeout", "transport", "unconfirmed"):
        return out
    out.update(content_type=content_type, body_bytes=raw["bytes"],
               transport_starts=raw["started"], transport_finishes=raw["finished"],
               fetch_settled=raw["settled"], response_complete=raw["complete"], failure=failure)
    value = None
    if raw["complete"]:
        try:
            value = comparison_json(body)
        except (ValueError, RecursionError):
            out["failure"] = failure or "invalid_json"
        if value is not None and content_type == "json":
            out["complete_json"], out["nonempty_text"] = comparison_complete(value, state["path"])
        if status == 403:
            out["refusal"] = comparison_refusal(body, value, raw["elapsed_ms"])
    out["accepted"] = (out["status"] == 200 and out["content_type"] == "json"
                       and out["response_complete"] and out["complete_json"]
                       and out["nonempty_text"] and raw["started"] == raw["finished"] == 1
                       and raw["settled"] and out["failure"] is None)
    if not out["accepted"] and out["failure"] is None:
        out["failure"] = "upstream" if status != 200 else "invalid_response"
    return out


async def comparison_baseline(host, account, scope, request):
    comparison_mark(scope, 'baseline')
    state = scope.comparison
    out = baseline_evidence(state)
    try:
        state, acc, header = await comparison_bound(host, account, scope, request, "baseline")
        if header != request["cookie_header"]:
            raise ValueError("comparison bearer jar changed")
        token = await comparison_mint(scope, state, acc)
        _, _, header = await comparison_bound(host, account, scope, request, "baseline")
        if header != request["cookie_header"]:
            raise ValueError("comparison bearer jar changed")
        args = {"page_url": state["url"], "device": state["device"],
                "path": state["path"], "body": state["baseline"], "token": token,
                "bearer": request["bearer"], "timeout_ms": BASELINE_SECONDS * 1000,
                "byte_limit": COMPARISON_BYTES}
        out.update(transport_starts=None, transport_finishes=None, fetch_settled=False)
        raw = await comparison_rpc(scope, lambda: acc.page.evaluate(COMPARISON_FETCH, args),
                                   BASELINE_SECONDS + 2)
        out = baseline_result(state, raw)
        del raw, args, token
        if not out["fetch_settled"]:
            scope.fail("comparison_fetch_unconfirmed")
        elif out["accepted"]:
            await comparison_bound(host, account, scope, request, "baseline")
            scope.comparison_next = "baseline_passed"
        return out
    except asyncio.CancelledError:
        scope.fail("comparison_cancelled")
        raise
    except Exception:
        out["accepted"] = False
        out["failure"] = "preparation" if out["fetch_settled"] else "unconfirmed"
        if not out["fetch_settled"]:
            scope.fail("comparison_fetch_unconfirmed")
        return out


async def comparison_command(host, account, request):
    try:
        validate_comparison_request(request)
        scope = host._observed.accounts[account]
        require_running(scope)
        pair = (request.get("cmd"), request.get("purpose"))
        actions = {
            ("prepare_inference_authorization", "baseline_context"):
                ("context_pending", comparison_context),
            ("browser_baseline_once", None): ("baseline_pending", comparison_baseline),
            ("prepare_inference_authorization", "native_token"):
                ("native_pending", comparison_native),
        }
        phase, action = actions[pair]
        if scope.comparison_phase != phase:
            raise ValueError("comparison phase mismatch")
        return await action(host, account, scope, request)
    except asyncio.CancelledError:
        raise
    except Exception:
        raise RuntimeError("browser comparison failed") from None
