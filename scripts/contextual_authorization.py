"""One direct wrapper call on the actual project page, with private cookies."""
import asyncio
import re
from urllib.parse import urlsplit

API = "https://api.framer.com"
ROUTES = frozenset(("/ai/v3/responses/", "/ai/v3/messages/"))
LIMIT = 16 * 1024


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
