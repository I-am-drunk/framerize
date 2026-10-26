"""Experimental shared browser that sends an already-authorized request."""
import asyncio
import json
import time
import uuid
from urllib.parse import urlsplit, unquote

from .app import Host
from .browser.driver import browser_runtime, BrowserCloseReceipt
from .resources import prepared_memory_browser, prepared_memory_addon
from .browser.responses import receive, BODY_LIMIT
from .supplied_scripts import START, POLL

ROUTES = frozenset(("/ai/v3/responses/", "/ai/v3/messages/",
                   "/ai/v3/openai/responses/", "/ai/v3/anthropic/messages/",
                   "/ai/v3/chat/"))


def validate(req):
    if req.get("version") != 1 or req.get("path") not in ROUTES:
        raise ValueError("unsupported supplied request")
    for key in ("account", "user", "device", "bearer", "cookie_header", "wire"):
        if type(req.get(key)) is not str or not req[key]:
            raise ValueError("missing supplied request field")
    if any(len(req[k].encode()) > 16384 for k in ("bearer", "cookie_header")):
        raise ValueError("supplied credential limit")
    if len(req["wire"].encode()) > 4 * 1024**2:
        raise ValueError("supplied request limit")
    for key, maximum in (("timeout_ms", 180000), ("response_limit", BODY_LIMIT)):
        if type(req.get(key)) is not int or not 0 < req[key] <= maximum:
            raise ValueError("invalid supplied request limit")
    if type(req.get("expires_unix_ms")) is not int:
        raise ValueError("missing supplied freshness")
    value = json.loads(req["wire"])
    if type(value) is not dict or not isinstance(value.get("castleToken"), str) or not value["castleToken"]:
        raise ValueError("missing supplied token")
    if len(value["castleToken"].encode()) > 16384:
        raise ValueError("supplied token limit")
    uuid.UUID(req["account"])
    if len(req["user"]) > 256 or len(req["device"]) > 256 or not all(
            c.isascii() and (c.isalnum() or c in "._~-") for c in req["device"]):
        raise ValueError("invalid supplied identity")
    if any(not 33 <= ord(c) <= 126 for c in req["bearer"]):
        raise ValueError("invalid supplied bearer")
    return request_cookies(req)


def request_cookies(req):
    values = {}
    for part in req["cookie_header"].split(";"):
        name, separator, value = part.strip().partition("=")
        if not separator or not name or name in values or not value or any(
                not 33 <= ord(c) <= 126 or c in '\\",;' for c in name + value):
            raise ValueError("invalid supplied cookies")
        values[name] = value
    if not values.get("session") or values.get("__cuid") != req["device"]:
        raise ValueError("supplied device or session mismatch")
    return [dict(name=k, value=v, domain=".framer.com" if k == "__cuid" else "api.framer.com",
                 path="/", secure=True, httpOnly=k == "session", sameSite="None")
            for k, v in values.items()]


class SuppliedHost(Host):
    def __init__(self):
        super().__init__()
        self.browser = self.receipt = self.start_task = None
        self.jobs = set()
        self.exchanges = set()
        self.uncertain = set()
        self.failed = False

    async def prepare(self):
        if self.start_task is None:
            self.start_task = asyncio.create_task(self._start_shared())
        await asyncio.shield(self.start_task)

    async def _start_shared(self):
        new_browser, playwright = browser_runtime()
        await self._start_driver(playwright)
        options = dict(headless=True, humanize=False, timeout=60000,
                       firefox_user_prefs={"dom.ipc.processCount": 1,
                           "dom.ipc.processCount.webIsolated": 1,
                           "browser.cache.memory.enable": False,
                           "browser.cache.disk.enable": False,
                           "browser.sessionhistory.max_entries": 2})
        executable, addon = prepared_memory_browser(), prepared_memory_addon()
        if executable:
            options["executable_path"] = executable
            options["firefox_user_prefs"].update({"framerize.captureResponseBodies": False,
                "framerize.captureRequestBodies": False, "framerize.reportActualRedirects": True})
        if addon:
            options.update(addons=[addon["path"]], exclude_addons=[addon["excluded_default"]])
        self.browser = await new_browser(self._playwright, **options)
        self.receipt = BrowserCloseReceipt(self.browser)

    async def handle(self, req):
        if self._closed or self.failed:
            raise RuntimeError("browser host is shutting down")
        if req.get("cmd") == "prepare_supplied":
            await self.prepare()
            return {"version": 1, "ready": True}
        if req.get("cmd") != "send_supplied":
            raise ValueError("unsupported supplied command")
        try:
            cookies = validate(req)
            if len(self.jobs) >= 10:
                raise ValueError("browser request capacity")
        except (ValueError, TypeError):
            return {"version": 1, "response": None, "cleanup_confirmed": True}
        cancelled = asyncio.Event()
        task = asyncio.create_task(self._run(req, cookies, cancelled))
        self.jobs.add(task)
        task.add_done_callback(self.jobs.discard)
        try:
            return await asyncio.shield(task)
        except asyncio.CancelledError:
            cancelled.set()
            return await asyncio.shield(task)

    async def _serve(self, req, writer):
        request_id = req.get("id")
        try:
            result = await self.handle(req)
        except BaseException:
            result = {"version": 1, "response": None, "cleanup_confirmed": False}
        req = None
        await writer.send({"id": request_id, "ok": True, "result": result})

    async def _run(self, req, cookies, cancelled):
        context = operation = waiter = None
        response, joined = None, True
        deadline = time.monotonic() + req["timeout_ms"] / 1000
        try:
            if cancelled.is_set() or time.time() * 1000 >= req["expires_unix_ms"]:
                raise ValueError("supplied token expired or cancelled")
            options = context_options(req.get("proxy"))
            joined = False
            await self.prepare()
            joined = True
            if cancelled.is_set() or time.time() * 1000 >= req["expires_unix_ms"]:
                raise ValueError("supplied token expired or cancelled")
            joined = False
            context = await self.browser.new_context(**options)
            joined = True
            self.uncertain.add(context)
            if cancelled.is_set():
                raise ValueError("supplied request cancelled")
            operation = asyncio.create_task(self._exchange(context, req, cookies, deadline))
            self.exchanges.add(operation)
            operation.add_done_callback(self.exchanges.discard)
            waiter = asyncio.create_task(cancelled.wait())
            done, _ = await asyncio.wait((operation, waiter),
                timeout=max(0, deadline - time.monotonic()), return_when=asyncio.FIRST_COMPLETED)
            if operation in done and not cancelled.is_set():
                response = operation.result()
        except (Exception, asyncio.CancelledError):
            response = None
        finally:
            if waiter is not None:
                waiter.cancel()
                await asyncio.gather(waiter, return_exceptions=True)
            if context is not None:
                close = asyncio.create_task(close_context(context))
                self.exchanges.add(close)
                close.add_done_callback(self.exchanges.discard)
                try:
                    await asyncio.wait_for(asyncio.shield(close), 10)
                    self.uncertain.discard(context)
                except (Exception, asyncio.CancelledError):
                    joined = False
            if operation is not None:
                try:
                    await asyncio.wait_for(asyncio.shield(operation), 10)
                except asyncio.TimeoutError:
                    # A completed exchange may itself raise ResponseDeadline.
                    # Only an exchange still running lacks a joined outcome.
                    if not operation.done():
                        joined = False
                except (Exception, asyncio.CancelledError):
                    pass
        if not joined:
            self.failed = True
        return {"version": 1, "response": response if joined else None, "cleanup_confirmed": joined}

    async def _exchange(self, context, req, cookies, deadline):
        async def route(r):
            parsed = urlsplit(r.request.url)
            document = r.request.resource_type == "document" and (
                parsed.scheme == "https" and parsed.netloc == "framer.com")
            inference = r.request.url == "https://api.framer.com" + req["path"]
            await (r.continue_() if document or inference else r.abort())
        await context.route("**/*", route)
        await context.add_cookies(cookies)
        await context.add_init_script("localStorage.setItem('__cuid', " + json.dumps(req["device"]) + ");")
        page = await context.new_page()
        await page.goto("https://framer.com/login", wait_until="domcontentloaded",
                        timeout=max(1, int((deadline - time.monotonic()) * 1000)))
        if urlsplit(page.url).netloc != "framer.com":
            raise ValueError("browser origin changed")
        job = uuid.uuid4().hex
        payload = {k: req[k] for k in ("path", "wire", "bearer", "device",
                   "expires_unix_ms", "response_limit", "timeout_ms")}
        await page.evaluate(START, [payload, job])
        content_type = None
        async def poll(ack):
            nonlocal content_type
            frame = await page.evaluate(POLL, [job, ack])
            if frame.get("content_type") is not None:
                content_type = frame["content_type"]
            return frame
        result = await receive(poll, deadline)
        result["content_type"] = content_type
        return result

    async def _close_host_active(self):
        self._closed = True
        if self.start_task is not None:
            await asyncio.gather(self.start_task, return_exceptions=True)
        if self.jobs:
            await asyncio.gather(*tuple(self.jobs), return_exceptions=True)
        if self.receipt is not None:
            await self.receipt.close()
        if self.exchanges:
            await asyncio.gather(*tuple(self.exchanges), return_exceptions=True)
        if self._playwright is not None:
            await self._playwright.stop()
        if self.start_task is not None and self.receipt is None:
            raise RuntimeError("browser startup cleanup unconfirmed")


async def close_context(context):
    closed = asyncio.Event()
    def observed(*_):
        closed.set()
    context.on("close", observed)
    try:
        await context._impl_obj._channel.send("close", None, {"reason": "supplied request finished"})
        await closed.wait()
    finally:
        context.remove_listener("close", observed)


def context_options(proxy):
    if proxy is None:
        return {}
    parsed = urlsplit(proxy)
    if parsed.scheme not in ("http", "https", "socks5") or not parsed.hostname or not parsed.port:
        raise ValueError("invalid account proxy")
    host = "[" + parsed.hostname + "]" if ":" in parsed.hostname else parsed.hostname
    value = {"server": f"{parsed.scheme}://{host}:{parsed.port}"}
    if parsed.username is not None:
        value.update(username=unquote(parsed.username), password=unquote(parsed.password or ""))
    return {"proxy": value}
