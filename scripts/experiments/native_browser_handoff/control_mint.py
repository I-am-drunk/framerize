"""Inert browser-control observation, never a live launcher.

The caller owns the page, account-generation check, tracked RPC and cleanup.
Cancellation or refusal does not release those owners. This module neither
uses Host._mint nor asserts which SDK branch the installed wrapper selected.
"""
import math
import time
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import urlsplit


class ObservationRefused(RuntimeError):
    def __init__(self):
        super().__init__("browser mint observation refused")


@dataclass(frozen=True, repr=False)
class ExpectedContext:
    user: str
    device: str
    session: str
    page_url: str
    wrapper_url: str
    post_path: str


@dataclass(frozen=True, repr=False)
class ObservedMint:
    token: str
    invocation_started: float
    received_at: float


def _validate(expected, deadline):
    if type(expected) is not ExpectedContext:
        raise ObservationRefused()
    for value in vars(expected).values():
        if type(value) is not str or not value or len(value.encode()) > 16384:
            raise ObservationRefused()
    page, wrapper = urlsplit(expected.page_url), urlsplit(expected.wrapper_url)
    if (page.scheme != "https" or page.netloc != "framer.com"
            or wrapper.scheme != "https" or not wrapper.netloc
            or wrapper.username is not None or wrapper.fragment):
        raise ObservationRefused()
    if expected.post_path not in ("/ai/v3/responses/", "/ai/v3/messages/",
            "/ai/v3/openai/responses/", "/ai/v3/anthropic/messages/", "/ai/v3/chat/"):
        raise ObservationRefused()
    if (type(deadline) not in (int, float) or not math.isfinite(deadline)
            or not 0 < deadline - time.monotonic() <= 180):
        raise ObservationRefused()


def _cookies_match(cookies, expected):
    if type(cookies) is not list or any(type(c) is not dict for c in cookies):
        return False
    for name, value in (("session", expected.session), ("__cuid", expected.device)):
        selected = [c.get("value") for c in cookies if c.get("name") == name]
        if selected != [value]:
            return False
    return True


def _current(page, context, expected, owner_current, deadline):
    if (owner_current() is not True or page.context is not context
            or page.is_closed() or page.url != expected.page_url
            or time.monotonic() >= deadline):
        raise ObservationRefused()


async def observe(page, expected, rpc, owner_current, deadline):
    """Use caller-tracked RPCs; the caller must join them after cancellation."""
    try:
        _validate(expected, deadline)
        context = page.context
        check = lambda: _current(page, context, expected, owner_current, deadline)
        check()
        await _check_cookies(context, expected, rpc, check)
        script = Path(__file__).with_suffix(".js").read_text()
        request = dict(user=expected.user, device=expected.device,
                       page_url=expected.page_url, wrapper_url=expected.wrapper_url)
        started = time.monotonic()
        result = await rpc(page.evaluate, script, request)
        received = time.monotonic()
        check()
        await _check_cookies(context, expected, rpc, check)
        return _result(result, started, received)
    except Exception:
        raise ObservationRefused() from None


async def _check_cookies(context, expected, rpc, check):
    for path in ("/web/users/me", expected.post_path):
        check()
        cookies = await rpc(context.cookies, "https://api.framer.com" + path)
        if not _cookies_match(cookies, expected):
            raise ObservationRefused()
        check()


def _result(result, started, received):
    if (type(result) is not dict or set(result) != {"version", "token", "matched"}
            or type(result["version"]) is not int or result["version"] != 1
            or type(result["matched"]) is not dict
            or set(result["matched"]) != {"user", "device", "page", "wrapper"}
            or any(v is not True for v in result["matched"].values())):
        raise ObservationRefused()
    token = result["token"]
    if type(token) is not str or not token or len(token.encode()) > 16384:
        raise ObservationRefused()
    return ObservedMint(token, started, received)
