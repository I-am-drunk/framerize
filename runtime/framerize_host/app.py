"""Browser host command and account lifecycle orchestration."""
import asyncio, base64, hashlib, json, os, random, shutil, sys, time, uuid
from contextlib import asynccontextmanager
from contextvars import ContextVar
from pathlib import Path
from .settings import (ROOT, LOGIN_URL, API, CASTLE_CHUNK, POOL_CAP, FORGE_MAX_AGE_S,
    DEVICE_POOL, HEARTBEAT_S, RESERVE_PER_DAY, DEV_COOLDOWN_S, DEV_WINDOW_S,
    DEV_WINDOW_CAP, MINT_GAP_S)
from .events import trace, emit
from framerize_runtime.wire import AsyncReplyWriter
from .resources import (memory_snapshot, trim_unused_heap, MemoryPressureError,
    BROWSER_STARTUP_RESERVE, prepared_memory_browser, prepared_memory_addon)
from .accounts.state import Device, AccountLifecycle, Account
from framerize_accounts.rates import AccountRates, RatePolicy, SystemClock
from .browser.driver import browser_runtime, BrowserCloseReceipt
from .browser.proxy import proxy_options
from .browser.session import set_account_cookie
from .browser.scripts import (WARM_JS, MINT_JS, post_js, AI_START_JS,
    AI_POLL_JS, AI_CANCEL_JS, DEVPOOL_WARM_JS, DEVPOOL_MINT_JS)
from .browser.actions import human_burst
from .browser.responses import receive as receive_response, ResponseDeadline
from . import castle_forge as _cf

_MINT_CALLS = ContextVar('framerize_mint_calls', default=None)

class Host:
    def __init__(self, rate_journal=None):
        os.makedirs(ROOT, exist_ok=True)
        self.rate_journal = rate_journal
        self.rate_clock = SystemClock()
        self.rate_policy = RatePolicy(window=DEV_WINDOW_S, cooldown=DEV_COOLDOWN_S,
            mint_gap=MINT_GAP_S, capacity=DEV_WINDOW_CAP, reserves=RESERVE_PER_DAY)
        self.accounts = {}  # account -> Account
        self.lifecycles = {}  # account -> lightweight history, never credentials
        self.generation = uuid.uuid4().hex
        self.proxies = {}
        self.proxy_revisions = {}
        self.lock = asyncio.Lock()  # brief: account-dict lifecycle ONLY
        self._playwright = None
        self._driver_start_task = None
        self._browser_runtime = None
        self._memory_browser = None
        self._memory_addon = None
        self._closed = False
        self._close_task = None
        self._allocation_lock = asyncio.Lock()
        self._reserved_bytes = 0
        self._command_tasks = {}
        self._reply_writer = None
        self._last_heap_trim = None
        # Closing idle browsers adds a cold start. Keep them warm by default.
        self.idle_timeout = float(os.environ.get("FRAMERIZE_IDLE_TIMEOUT_S", "0"))
        configured_gc = os.environ.get("FRAMERIZE_IDLE_GC_S")
        # An explicit interval applies in both modes; only defaults accelerate.
        self.idle_gc_interval = float("120" if configured_gc is None else configured_gc)
        self.idle_gc_pressure_interval = float("60" if configured_gc is None else configured_gc)

    @asynccontextmanager
    async def _activity(self, account):
        life = self._lifecycle(account)
        with life.activity.activity():
            yield

    def _lifecycle(self, account):
        life = self.lifecycles.get(account)
        if life is None:
            rates = AccountRates(self.rate_journal, account, self.rate_policy, self.rate_clock)
            life = AccountLifecycle(rates)
            self.lifecycles[account] = life
        return life

    async def _rpc(self, account, call, *args, **kwargs):
        life = self.lifecycles[account]
        activity = life.activity.activity()
        async def actual():
            try:
                return await call(*args, **kwargs)
            finally:
                activity.close()
        task = life.activity.track(actual)
        owned = _MINT_CALLS.get()
        if owned is not None:
            owned.add(task)
            task.add_done_callback(owned.discard)
        # Cancellation stops the waiter; the raw RPC keeps its account owner.
        return await asyncio.shield(task)

    def _check_proxy_revision(self, req):
        account = req.get("account", "")
        revision = req.get("proxy_revision", 0)
        if type(revision) is not int or not 0 <= revision <= 2**64 - 1:
            raise ValueError("invalid account proxy revision")
        current = self.proxy_revisions.get(account)
        if current is not None and revision != current:
            raise RuntimeError("stale account proxy revision")
        return revision

    def _adopt_proxy_route(self, req):
        account = req.get("account", "")
        revision = self._check_proxy_revision(req)
        desired = req.get("proxy", self.proxies.get(account))
        if desired is not None and (not isinstance(desired, str) or not desired):
            raise ValueError("proxy must be a nonempty string or null")
        if desired != self.proxies.get(account) and (account in self.proxy_revisions
                or self._browser_status(account)["state"] != "absent"):
            raise RuntimeError("account proxy change requires confirmed browser closure and registration")
        self.proxy_revisions[account] = revision
        if desired is None:
            self.proxies.pop(account, None)
        else:
            self.proxies[account] = desired

    def _register_proxy(self, req):
        account = req["account"]
        expected, revision, proxy = req["expected_revision"], req["revision"], req["proxy"]
        if (type(expected) is not int or type(revision) is not int
                or not 0 <= expected < revision <= 2**64 - 1 or revision != expected + 1):
            raise ValueError("invalid account proxy registration revision")
        if proxy is not None and (not isinstance(proxy, str) or not proxy):
            raise ValueError("proxy must be a nonempty string or null")
        def reply(outcome):
            return {"outcome": outcome, "status": self._browser_status(account),
                    "route": {"revision": self.proxy_revisions.get(account, 0),
                              "proxy": self.proxies.get(account)}}
        status = self._browser_status(account)
        if status["state"] != "absent" or status["active_commands"]:
            return reply("busy")
        current = self.proxy_revisions.get(account)
        if current == revision and self.proxies.get(account) == proxy:
            return reply("registered")
        if current is not None and current != expected:
            return reply("stale")
        self.proxy_revisions[account] = revision
        if proxy is None:
            self.proxies.pop(account, None)
        else:
            self.proxies[account] = proxy
        return reply("registered")

    @asynccontextmanager
    async def _allocation(self, estimate):
        async with self._allocation_lock:
            snapshot = memory_snapshot()
            available = snapshot.get("allocation_headroom_bytes")
            if available is not None and available < self._reserved_bytes + estimate:
                emit("memory.admission", level="warn", outcome="deferred", **snapshot)
                raise MemoryPressureError("browser allocation deferred at the 90% memory watermark; retry after memory is available")
            self._reserved_bytes += estimate
        try:
            yield
        finally:
            self._reserved_bytes -= estimate

    @staticmethod
    def _track_task(acc, coro, *, page=None, idle_loop=False):
        activity = None
        try:
            if not idle_loop:
                activity = acc.lifecycle.activity.activity()
            task = asyncio.create_task(coro)
        except BaseException:
            coro.close()
            if activity is not None:
                activity.close()
            raise
        if activity is not None:
            def completed(done):
                if not done.cancelled():
                    done.exception()
                activity.close()
            task.add_done_callback(completed)
        acc.tasks.add(task)
        task.add_done_callback(acc.tasks.discard)
        if page is not None:
            # A sleeping background task otherwise retains its closed page
            # until the next timer wake. Only cancel this page's own task.
            owner_task = task

            def page_closed(*_):
                if owner_task is not None:
                    owner_task.cancel()

            def detach(_=None):
                nonlocal owner_task
                # A damaged emitter may fail to remove the callback. Make
                # any residual callback inert without retaining the task.
                owner_task = None
                try:
                    page.remove_listener("close", page_closed)
                except Exception:
                    pass

            try:
                page.on("close", page_closed)
            except Exception:
                # Optional lifecycle tracking must not break live work.
                # Also remove a hook if registration only partly succeeded.
                detach()
            else:
                task.add_done_callback(detach)
                try:
                    # Covers a page closed before the listener was attached.
                    if page.is_closed():
                        page_closed()
                except Exception:
                    # Future close events still provide cleanup; retain the
                    # original polling lifetime if inspection is unavailable.
                    pass
        return task

    @asynccontextmanager
    async def _use_account(self, account, proxy=None):
        async with self._activity(account):
            async with self._use_account_active(account, proxy) as acc:
                yield acc

    @asynccontextmanager
    async def _use_account_active(self, account, proxy=None):
        life = self._lifecycle(account)
        life.active_commands += 1
        acc = None
        try:
            current = await self._account(account, proxy)
            if current.closing:
                raise RuntimeError("account browser is closing; retry after cleanup")
            acc = current
            acc.active_requests += 1
            acc.last_used = time.monotonic()
            yield acc
        finally:
            life.active_commands -= 1
            if acc is not None:
                acc.active_requests -= 1
                acc.last_used = time.monotonic()

    async def _dispose_account(self, acc, account):
        acc.closing = True
        tasks = tuple(acc.tasks)
        for task in tasks:
            task.cancel()
        if tasks:
            await asyncio.gather(*tasks, return_exceptions=True)
        pending = tuple(acc.lifecycle.activity.pending)
        if pending:
            await asyncio.gather(*(asyncio.shield(task) for task in pending),
                                 return_exceptions=True)
        # Browser closure includes every context. Other accounts keep their
        # separate browsers and continue using the shared protocol driver.
        close_receipt = acc.lifecycle.close_receipt
        async def close_browser():
            await close_receipt.close()
        await asyncio.wait_for(self._rpc(account, close_browser), 10)
        acc.dev_objs.clear()
        acc.token_pool.clear()
        acc.forge_parts.clear()
        acc.session_cookie = None
        acc.borrowed.clear()

    def _browser_status(self, account):
        life = self.lifecycles.get(account)
        if life is not None and life.phase == "absent" and (
                account in self.accounts or life.browser is not None or life.close_receipt is not None):
            life.phase = "unconfirmed"
        if life is not None and life.phase == "resident":
            try:
                acc = self.accounts.get(account)
                connected = (acc is not None and acc.browser is life.browser
                             and life.browser.is_connected())
            except Exception:
                connected = False
            if not connected:
                life.phase = "unconfirmed"
                if account in self.accounts:
                    self.accounts[account].closing = True
        return {"account_id": account, "host_generation": self.generation,
                "account_incarnation": 0 if life is None else life.incarnation,
                "state": ("unconfirmed" if account in self.accounts else "absent") if life is None else life.phase,
                "active_commands": 0 if life is None else life.active_commands}

    async def _close_idle(self, req):
        account = req["account"]
        current = self._browser_status(account)
        if (not isinstance(req.get("host_generation"), str)
                or type(req.get("account_incarnation")) is not int):
            raise ValueError("idle close requires the observed browser identity")
        if any(req[key] != current[key] for key in ("host_generation", "account_incarnation")):
            return {"outcome": "stale_generation", "status": current}
        if current["active_commands"] or current["state"] == "starting":
            return {"outcome": "busy", "status": current}
        if current["state"] == "absent":
            return {"outcome": "already_absent", "status": current}
        closed = await self._close_account(account)
        return {"outcome": "closed" if closed else "unconfirmed",
                "status": self._browser_status(account)}

    async def _close_account(self, account, *, idle_before=None):
        # Includes monitor-initiated closure, not just command dispatch.
        async with self._activity(account):
            return await self._close_account_active(account, idle_before=idle_before)

    async def _close_account_active(self, account, *, idle_before=None):
        life = self.lifecycles.get(account)
        if life is None:
            return account not in self.accounts
        acc = self.accounts.get(account)
        if life.active_commands or life.phase == "starting":
            return False
        if life.activity.pending and life.close_task is None:
            return False
        if acc is not None and (acc.active_requests or
                (idle_before is not None and acc.last_used >= idle_before)):
            return False
        if life.phase == "absent":
            return acc is None and life.browser is None and life.close_receipt is None
        if life.close_task is None:
            self._start_close(account, life, acc)
        return await asyncio.shield(life.close_task)

    def _start_close(self, account, life, acc):
        activity = life.activity.activity()
        life.phase = "closing"
        if acc is not None:
            acc.closing = True
        registered = asyncio.get_running_loop().create_future()
        async def finish():
            try:
                await registered
                return await self._finish_close(account, life, acc)
            finally:
                activity.close()
        runner = finish()
        try:
            life.close_task = asyncio.create_task(runner)
        except BaseException:
            runner.close()
            activity.close()
            life.phase = "unconfirmed"
            raise
        registered.set_result(None)
        return life.close_task

    async def _finish_close(self, account, life, acc):
        incarnation = life.incarnation
        try:
            if life.browser is None:
                raise RuntimeError("browser launch cleanup is unconfirmed")
            if life.close_receipt is None or life.close_receipt.browser is not life.browser:
                raise RuntimeError("browser close ownership is unconfirmed")
            if acc is not None:
                await self._dispose_account(acc, account)
            else:
                await asyncio.wait_for(self._rpc(account, life.close_receipt.close), 10)
            if (self.lifecycles.get(account) is not life or life.incarnation != incarnation
                    or self.accounts.get(account) is not acc):
                raise RuntimeError("browser ownership changed during cleanup")
            life.rates.confirm_runtime_stopped()
            self.accounts.pop(account, None)
            life.browser = None
            life.close_receipt = None
            life.phase = "absent"
            return True
        except asyncio.CancelledError:
            life.phase = "unconfirmed"
            raise
        except Exception as exc:
            life.phase = "unconfirmed"
            emit("browser.cleanup", level="warn", outcome="unconfirmed",
                 error=type(exc).__name__)
            return False
        finally:
            life.close_task = None

    async def _monitor_once(self):
        snapshot = memory_snapshot()
        emit("memory.snapshot", accounts=len(self.accounts),
             active_requests=sum(a.active_requests for a in self.accounts.values()),
             background_tasks=sum(len(a.tasks) for a in self.accounts.values()),
             pages=sum(len(c.pages) for a in self.accounts.values() for c in a.browser.contexts),
             **snapshot)
        for account, life in list(self.lifecycles.items()):
            acc = self.accounts.get(account)
            if life.phase in ("closing", "unconfirmed") or (acc is not None and self.idle_timeout > 0):
                try:
                    cutoff = None if life.phase != "resident" else time.monotonic() - self.idle_timeout
                    await self._close_account(account, idle_before=cutoff)
                except Exception as exc:
                    emit("browser.cleanup", level="warn", outcome="error", error=type(exc).__name__)
        await self._reclaim_idle_memory(snapshot)
        self._trim_idle_heap(snapshot)

    def _trim_idle_heap(self, snapshot):
        """Release spare host heap pages only between commands under pressure."""
        headroom = snapshot.get("allocation_headroom_bytes")
        now = time.monotonic()
        if (self.idle_gc_interval <= 0 or headroom is None
                or headroom >= BROWSER_STARTUP_RESERVE
                or self._command_tasks or self._reserved_bytes
                or (self._reply_writer is not None and self._reply_writer.busy)
                or self.lock.locked() or self._allocation_lock.locked()
                or (self._last_heap_trim is not None
                    and now - self._last_heap_trim < self.idle_gc_interval)):
            return
        for acc in self.accounts.values():
            if (acc.closing or acc.active_requests or acc.borrowed
                    or acc.mint_lock.locked()
                    or any(d.lock.locked() for d in acc.dev_objs.values())
                    or any(t.get("in_use") for t in acc.token_pool.values())):
                return
        self._last_heap_trim = now
        outcome, released = "ok", False
        try:
            result = trim_unused_heap()
            if result is None:
                outcome = "unsupported"
            else:
                released = result
        except Exception:
            outcome = "unavailable"
        emit("memory.host_trim", outcome=outcome, released=released,
             duration_ms=round((time.monotonic() - now) * 1000, 3))

    async def _reclaim_idle_memory(self, snapshot):
        """Reclaim unreachable objects without unloading a warm account.

        At most three idle accounts are collected per tick, or six when the
        reserve is breached. Stop starting further work after 250 ms;
        a started collection may take longer.
        This does not clear
        cookies, storage, handles or live JS objects, and never retries a stuck
        collection in a loop. Regular requests do not wait on a host lock.
        """
        headroom = snapshot.get("allocation_headroom_bytes")
        # Start reclamation when a new browser would be refused by admission.
        # The former 256 MiB trigger left idle caches untouched while a launch
        # needed 1 GiB of headroom. Existing idle and busy checks still apply.
        if (self.idle_gc_interval <= 0 or headroom is None
                or headroom >= BROWSER_STARTUP_RESERVE):
            return
        started = time.monotonic()
        collections = 0
        pressure = headroom < 0
        cooldown = self.idle_gc_pressure_interval if pressure else self.idle_gc_interval
        collection_limit = 6 if pressure else 3
        candidates = sorted(self.accounts.items(), key=lambda item: item[1].last_gc)
        for account, acc in candidates:
            now = time.monotonic()
            if now - started >= 0.250:
                return
            if (acc.closing or acc.active_requests or now - acc.last_used < 30
                    or now - acc.last_gc < cooldown
                    or acc.mint_lock.locked() or acc.borrowed
                    or any(d.lock.locked() for d in acc.dev_objs.values())
                    or any(t.get("in_use") for t in acc.token_pool.values())):
                continue
            pages = [p for ctx in acc.browser.contexts for p in ctx.pages if not p.is_closed()]
            if not pages:
                continue
            async with self._activity(account):
                page = pages[acc.gc_page % len(pages)]
                acc.gc_page += 1
                previous_gc = acc.last_gc
                acc.last_gc = now
                outcome = "ok"
                try:
                    # Heartbeats also use these pages. Inspect all contexts without
                    # modifying jobs, then recheck activity after the async probe.
                    pending = await asyncio.wait_for(asyncio.gather(*(self._rpc(account, p.evaluate,
                        "Object.values(window.__aiJobs || {}).some(job => !job.done)"
                    ) for p in pages)), timeout=0.5)
                    if (any(pending) or acc.closing or acc.active_requests
                            or time.monotonic() - acc.last_used < 30
                            or acc.mint_lock.locked() or acc.borrowed
                            or any(d.lock.locked() for d in acc.dev_objs.values())
                            or any(t.get("in_use") for t in acc.token_pool.values())):
                        outcome = "busy"
                        acc.last_gc = previous_gc
                    else:
                        await asyncio.wait_for(self._rpc(account, page.request_gc), timeout=1)
                except asyncio.CancelledError:
                    raise
                except Exception:
                    outcome = "unavailable"
                emit("memory.idle_gc", account=account[:8], outcome=outcome, pressure=pressure,
                     duration_ms=int((time.monotonic() - now) * 1000))
                if outcome == "ok":
                    collections += 1
                if outcome == "unavailable" or collections >= collection_limit:
                    return

    async def _monitor(self):
        while True:
            await asyncio.sleep(30)
            await self._monitor_once()

    async def close(self):
        if self._close_task is None:
            self._closed = True
            self._close_task = asyncio.create_task(self._close_host_active())
        return await asyncio.shield(self._close_task)

    async def _close_host_active(self):
        self._closed = True
        commands = tuple(self._command_tasks.values())
        if commands:
            await asyncio.gather(*commands, return_exceptions=True)
        if self._driver_start_task is not None:
            await asyncio.gather(self._driver_start_task, return_exceptions=True)
        background = [task for acc in self.accounts.values() for task in acc.tasks]
        for task in background:
            task.cancel()
        if background:
            await asyncio.gather(*background, return_exceptions=True)
        pending = [task for life in self.lifecycles.values()
                   for task in life.activity.pending]
        if pending:
            await asyncio.gather(*(asyncio.shield(task) for task in pending),
                                 return_exceptions=True)
        unconfirmed = []
        for account in list(self.lifecycles):
            try:
                if not await self._close_account(account):
                    unconfirmed.append(account)
            except Exception:
                unconfirmed.append(account)
        if unconfirmed:
            raise RuntimeError("browser shutdown could not confirm every account closed")
        if self._playwright is not None:
            await self._playwright.stop()
            self._playwright = None
        self.accounts.clear()

    async def _human_loop(self, account, page):
        """Continuous human presence on one page: a small activity burst every
        ~25-90s (randomized). Dies with the page."""
        rng = random.Random()
        while True:
            await asyncio.sleep(rng.uniform(25, 90))
            try:
                if page.is_closed():
                    return
                async with self._activity(account):
                    await human_burst(page, rng, rpc=lambda call, *args, **kwargs:
                        self._rpc(account, call, *args, **kwargs))
            except Exception:
                return

    async def _acquire(self, account, proxy=None):
        """Returns (page, acc, queue_wait_s). queue_wait_s is the time spent
        waiting for a free page — NOT upstream latency. Callers report it so the
        chunk sizer can subtract it: queue wait and a genuinely slow upstream
        look identical in wall-clock, and conflating them makes the sizer shrink
        chunks under load, which multiplies round trips and worsens the queue."""
        acc = await self._account(account, proxy)
        async with acc.lock:
            while not acc.pool.empty():
                page = acc.pool.get_nowait()
                if page.is_closed():
                    acc.size -= 1
                    continue
                acc.borrowed.add(page)
                return page, acc, 0.0
            if acc.size < POOL_CAP:
                async with self._allocation(64 * 1024**2):
                    page = await self._rpc(account, acc.ctx.new_page)
                    try:
                        await self._rpc(account, page.goto, LOGIN_URL, wait_until="domcontentloaded", timeout=60000)
                        await self._rpc(account, page.wait_for_timeout, 800)
                    except BaseException:
                        await self._rpc(account, page.close)
                        raise
                acc.size += 1
                acc.borrowed.add(page)
                self._track_task(acc, self._human_loop(account, page), page=page, idle_loop=True)
                return page, acc, 0.0
        # At cap: this request QUEUES for a free page. Log the wait — queue
        # time dominates total latency under concurrent load (47s→190s
        # arithmetic growth was invisible until measured here).
        t0 = time.monotonic()
        page = await acc.pool.get()
        if page.is_closed():
            acc.size -= 1
            return await self._acquire(account, proxy)
        acc.borrowed.add(page)
        wait = time.monotonic() - t0
        if wait > 2.0:
            trace(f"{account[:8]}: pool wait {wait:.1f}s (all {POOL_CAP} pages busy)")
        return page, acc, wait

    async def _ensure_editor(self, acc, page, account):
        """Keep the page resident on the account's project EDITOR, not /login.

        INVISIBILITY (2026-10-05): every AI fetch carries the page's Referer
        and every Castle token embeds its page context — minting and firing
        generations from framer.com/login marked every request as "not a real
        editing session" (no logged-in user sits on the login page all day).
        A real agent user has the editor open: tokens minted there, calls
        fired from there, ambient sync/presence traffic flowing. After any
        navigation the castle module cache is gone, so re-warm it (and the
        device pool) — one-time ~6-10s per page, then fast.
        """
        if not acc.editor_url:
            return
        # 2026-10-11, measured on a FRESH profile: fires from the editor 403,
        # fires from /login 200. The residency theory holds for long-lived
        # established pages (iris), but a profile that JUST landed on the
        # editor and instantly calls AI reads as automation to Castle.
        # FRAMERIZE_NO_EDITOR=1 skips residency entirely (iMac default until
        # profiles mature).
        if os.environ.get("FRAMERIZE_NO_EDITOR") == "1":
            return
        try:
            current = page.url or ""
        except Exception:
            current = ""
        # The editor SPA redirects /projects/<id> to the slugged
        # /projects/<name>--<id> — a startswith() check on the bare-id URL
        # NEVER matched, so every ai_post re-navigated (52 in 10 min, chains
        # stalling 10-90s per chunk — "sessions not moving"). Match the id
        # anywhere in the URL instead.
        pid = acc.editor_url.rsplit('/', 1)[-1]
        if pid and pid in current:
            return
        try:
            await self._rpc(account, page.goto, acc.editor_url, wait_until="domcontentloaded", timeout=90000)
            trace(f"{account[:8]}: page -> editor ({acc.editor_url.split('/')[-1][:12]}...)")
            await self._rpc(account, page.wait_for_timeout, 1500)
            await self._rpc(account, page.evaluate, WARM_JS)
            if acc.devices:
                try:
                    await self._rpc(account, page.evaluate, DEVPOOL_WARM_JS.replace('%s', json.dumps(acc.devices)))
                except Exception:
                    pass
        except Exception as e:
            trace(f"{account[:8]}: editor navigation issue ({type(e).__name__}) — continuing")

    def _set_editor(self, acc, account, project_id):
        url = f"https://framer.com/projects/{project_id}"
        if acc.editor_url == url:
            return
        acc.editor_url = url
        if acc.editor_file:
            try:
                Path(acc.editor_file).write_text(json.dumps({"editor_url": url}))
            except Exception:
                pass

    @staticmethod
    def _release(acc, page):
        if page not in acc.borrowed:
            return
        acc.borrowed.remove(page)
        if acc.closing:
            acc.size -= 1
        else:
            # A closed page also wakes a waiter, which replaces it under the
            # creation lock. Dropping it here could strand every pool waiter.
            acc.pool.put_nowait(page)

    async def _new_mint_page(self, acc, account):
        """Create the account's dedicated mint page and warm it.

        WHY THIS PAGE EXISTS (measured 2026-10-04): a warm v2 mint costs ~25ms
        on an idle page, but mints measured p50=46s (max 166s) under load.
        Mints ran on the SAME pool pages as 100s-generation fetches, and all
        pages sit on framer.com/login — one Fission content process, one JS
        main thread, browser-serialized work. Every mint queued behind
        in-flight generations and vice versa. The "upstream collapse" was
        this contention, never the upstream (direct probes: 119 tok/s the
        same minute). A page that ONLY mints has an always-free main thread:
        mints stay ~25ms forever."""
        async with self._allocation(64 * 1024**2):
            page = await self._rpc(account, acc.ctx.new_page)
            try:
                await self._rpc(account, page.goto, LOGIN_URL, wait_until="domcontentloaded", timeout=60000)
                await self._rpc(account, page.wait_for_timeout, 800)
                await self._rpc(account, page.evaluate, WARM_JS)
                if acc.devices:
                    try:
                        await self._rpc(account, page.evaluate, DEVPOOL_WARM_JS.replace('%s', json.dumps(acc.devices)))
                    except Exception:
                        pass
            except BaseException:
                await self._rpc(account, page.close)
                raise
        self._track_task(acc, self._human_loop(account, page), page=page, idle_loop=True)
        trace(f"{account[:8]}: dedicated mint page ready")
        return page

    TOKEN_TTL_S = 480.0  # 2026-10-05: TTL enforcement silently turned ON (~600s
                         # boundary measured via the velocity-cooling cascade — old
                         # tokens 403 and were misread as velocity bans). 480 = safe margin.
    TOKEN_CONTENT_MAX_AGE_S = 1800.0  # re-wrap hygiene: re-mint the base after 30min even though server TTL is unenforced
    TOKEN_MAX_USES = 5   # contract 2026-10-05: a token is a 6-USE coupon — use #7
                         # returns 403 deterministically (verified: #1-6 pass, #7+ all
                         # 403). Retire at 5 to stay under. The old per-chunk-mint
                         # design was SAFE by accident; unbounded pooling was not.
    TOKEN_POOL_CAP = 8

    async def _dev_page(self, acc, account, dev):
        """Get-or-create the device's isolated context + page, with __cuid
        seeded BEFORE the castle module loads (add_init_script runs ahead of
        any page script) and the account session cookie fanned in. The module
        is warmed once per context (the v3-race latch is per module scope)."""
        d = acc.dev_objs.get(dev)
        if d is None:
            d = Device(dev)
            acc.dev_objs[dev] = d
        # Single-flight initialization keeps simultaneous first requests from
        # overwriting and leaking each other's context.
        async with d.lock:
            return await self._prepare_device(acc, account, d)

    async def _prepare_device(self, acc, account, d):
        dev = d.id
        if d.page is not None and d.page.is_closed():
            if d.ctx is not None:
                await self._rpc(account, d.ctx.close)
            d.page, d.ctx, d.warm = None, None, False
        if d.page is not None and d.warm:
            return d
        async with self._allocation(64 * 1024**2):
            if d.page is None:
                ctx = await self._rpc(account, acc.browser.new_context)
                try:
                    await self._rpc(account, ctx.add_init_script,
                        f"try {{ localStorage.setItem('__cuid', {json.dumps(dev)}); }} catch(e) {{}}"
                    )
                    cookies = [{"name": "__cuid", "value": dev, "domain": ".framer.com",
                                "path": "/", "secure": True, "httpOnly": False, "sameSite": "None"}]
                    if acc.session_cookie:
                        name, value = acc.session_cookie
                        cookies.append({"name": name, "value": value, "domain": ".framer.com",
                                        "path": "/", "secure": True, "httpOnly": True, "sameSite": "None"})
                    await self._rpc(account, ctx.add_cookies, cookies)
                    page = await self._rpc(account, ctx.new_page)
                    await self._rpc(account, page.goto, LOGIN_URL, wait_until="domcontentloaded", timeout=90000)
                except BaseException:
                    await self._rpc(account, ctx.close)
                    raise
                d.ctx, d.page = ctx, page
                trace(f"{account[:8]}: device {dev[:8]} context ready (isolated, pre-seeded)")
            if not d.warm:
                try:
                    await self._rpc(account, d.page.evaluate, WARM_JS)
                    d.warm = True
                except Exception as e:
                    trace(f"{account[:8]}: device {dev[:8]} warm issue ({type(e).__name__})")
        return d

    def _pick_device_sticky(self, acc):
        return acc.lifecycle.rates.select_primary()

    def _pick_device(self, acc, account=None):
        if not acc.devices:
            return None
        ready = acc.lifecycle.rates.select_primary()
        return ready if ready is not None else acc.lifecycle.rates.allocate_reserve()

    def _device_ready_in(self, acc):
        return acc.lifecycle.rates.device_ready_in() or None

    def _note_request(self, acc, dev):
        acc.lifecycle.rates.charge_request(dev)

    def _cool_device(self, acc, dev, account):
        acc.lifecycle.rates.cool_device(dev)
        # burn the device's outstanding coupons: a banned device's tokens all
        # draw from its drained velocity budget
        for tid in [k for k, v in acc.token_pool.items() if v.get("dev") == dev]:
            del acc.token_pool[tid]
        trace(f"{account[:8]}: device {str(dev)[:8]} velocity-cooling {int(DEV_COOLDOWN_S)}s (rotating)")

    @staticmethod
    def _rewrap_token(token):
        """Outer-layer re-wrap: decode the outer XOR with the embedded key byte,
        re-XOR with a fresh random byte. New token STRING, byte-identical content.
        VERIFIED 2026-10-05 (the Castle goose): the server's 6-use coupon tracker
        keys on the STRING — a burned coupon re-wrapped passes 200, and each
        re-wrapped string gets its own 6 uses. Pure python, no browser eval:
        one real mint now yields unlimited coupons."""
        raw = base64.urlsafe_b64decode(token + "=" * (-len(token) % 4))
        k1 = raw[0]
        body = bytes(b ^ k1 for b in raw[1:])
        k2 = random.randint(1, 255)
        while k2 == k1:
            k2 = random.randint(1, 255)
        return base64.urlsafe_b64encode(bytes([k2]) + bytes(b ^ k2 for b in body)).decode().rstrip("=")

    async def _token_acquire(self, acc, page, account, dev=None):
        """Castle token pool, device-aware (velocity budgets key on the token's
        embedded CUID): prefers idle coupons minted on the REQUESTED device,
        else mints on it. Sequential reuse is free; concurrent reuse of one
        token is the only deny signature (in_use flips atomically before any
        await in the reuse path — asyncio single-thread)."""
        now = time.monotonic()
        for tid, ent in list(acc.token_pool.items()):
            if now - ent.get("born", ent["ts"]) > self.TOKEN_TTL_S:
                # 2026-10-05: Framer silently enabled TTL enforcement (~600s; the
                # velocity-cooling cascade was old tokens 403ing, misread as
                # velocity bans). Retire well under the boundary.
                del acc.token_pool[tid]
                continue
            if dev is not None and ent.get("dev") != dev:
                continue  # stay on the velocity-selected device
            if ent["uses"] >= self.TOKEN_MAX_USES:
                # Coupon spent — RE-WRAP instead of re-minting: the tracker keys
                # on the token string, so a fresh string = a fresh 6-use coupon
                # for byte-identical content (verified 2026-10-05).
                ent["token"] = self._rewrap_token(ent["token"])
                ent["uses"] = 0
                ent["ts"] = now
                ent["wraps"] = ent.get("wraps", 0) + 1
                trace(f"{account[:8]}: token re-wrapped (wrap #{ent['wraps']} dev {str(ent.get('dev'))[:8]}) — fresh coupon, no mint")
            if not ent["in_use"]:
                ent["in_use"] = True
                ent["uses"] += 1
                trace(f"{account[:8]}: token reuse #{ent['uses']} dev {str(ent.get('dev'))[:8]} (age {now - ent['ts']:.0f}s, pool {len(acc.token_pool)})")
                return tid, ent["token"], ent.get("dev")
        if len(acc.token_pool) >= self.TOKEN_POOL_CAP:
            tok, used_dev = await self._mint(acc, page, account, dev)  # all busy: transient
            return None, tok, used_dev
        tok, used_dev = await self._mint(acc, page, account, dev)
        if not tok:
            return None, None, None
        tid = uuid.uuid4().hex[:12]
        acc.token_pool[tid] = {"token": tok, "ts": now, "born": now, "in_use": True, "uses": 1, "dev": used_dev}
        return tid, tok, used_dev

    def _token_release(self, acc, tid):
        if tid is None:
            return
        ent = acc.token_pool.get(tid)
        if ent:
            ent["in_use"] = False

    @asynccontextmanager
    async def _mint_gate(self, acc):
        await acc.mint_lock.acquire()
        started, owned = False, set()
        token = _MINT_CALLS.set(owned)
        try:
            delay = acc.lifecycle.rates.mint_ready_in()
            if delay > 0:
                await asyncio.sleep(delay)
            acc.lifecycle.rates.begin_mint()
            started = True
            yield
        finally:
            _MINT_CALLS.reset(token)
            if not started:
                acc.mint_lock.release()
            else:
                async def finish():
                    try:
                        await asyncio.gather(*(asyncio.shield(t) for t in tuple(owned)),
                                             return_exceptions=True)
                        acc.lifecycle.rates.finish_mint()
                    finally:
                        acc.mint_lock.release()
                try:
                    completion = acc.lifecycle.activity.track(finish)
                except BaseException:
                    acc.mint_lock.release()
                    raise
                await asyncio.shield(completion)

    async def _mint(self, acc, page, account, dev=None):
        """Mint one Castle token, serialized and paced per ACCOUNT.

        All of an account's pages share one device id (__cuid), so simultaneous
        mints look like one device firing N fingerprint requests at once --
        the burst signature a risk engine punishes. The lock makes mints
        one-at-a-time and MINT_GAP_S spaces them, so the device's request
        pattern stays human-plausible no matter how many agents are running.

        This is PREVENTION. A 403 risk-refusal must never be retried: another
        mint from an already-refused device is what turns a transient refusal
        into a sticky one.
        """
        # FORGE-FIRST (2026-10-05): if we hold fresh harvested parts for the
        # requested device, mint locally — no browser eval, no pacing gap, no
        # SDK race. Parts come from any real browser mint (harvested below)
        # and are refreshed every FORGE_MAX_AGE_S.
        if _cf is not None:
            ent = None
            fdev = dev
            if dev is not None:
                ent = acc.forge_parts.get(dev)
            else:
                fresh = [(e["born"], c) for c, e in acc.forge_parts.items()]
                if fresh:
                    fdev = max(fresh)[1]
                    ent = acc.forge_parts[fdev]
            if ent and (time.monotonic() - ent["born"]) < FORGE_MAX_AGE_S:
                try:
                    tok = _cf.forge(ent["parts"], fdev)
                    emit("castle.forge", account=account[:8], device=str(fdev)[:8], outcome="ok")
                    return tok, fdev
                except Exception:
                    pass  # fall through to a live mint
        async with self._mint_gate(acc):
            t0 = time.monotonic()
            tok = None
            burst_ms = 0  # pre-mint burst REMOVED: it was the bottleneck
            # (19-21s under humanize=True). The per-page humanize loops keep
            # Castle's behavioral buffers warm between mints.
            used_dev = dev
            if dev is not None and acc.browser is not None:
                # CONTEXT-PER-DEVICE: mint inside the device's own isolated
                # context. The SDK binds the context's single pre-seeded
                # __cuid — no swaps, no shared-page churn, ever.
                try:
                    d = await self._dev_page(acc, account, dev)
                    tok = await self._rpc(account, d.page.evaluate, MINT_JS)
                    used_dev = dev
                except Exception as e:
                    trace(f"{account[:8]}: context mint failed on {str(dev)[:8]} ({type(e).__name__})")
                    tok = None
            if not tok:
                page = acc.mint_page
                if page is None or page.is_closed():
                    page = acc.mint_page = await self._new_mint_page(acc, account)
                tok = await self._rpc(account, page.evaluate, MINT_JS)
                used_dev = None  # single-device path (ambient __cuid)
            if tok and _cf is not None:
                try:
                    parts = _cf.harvest(tok)
                    acc.forge_parts[parts["cuid"]] = {"parts": parts, "born": time.monotonic()}
                except Exception:
                    pass
            emit("castle.mint", account=account[:8],
                 duration_ms=int((time.monotonic() - t0) * 1000),
                 burst_ms=burst_ms, device=str(used_dev)[:8] if used_dev else None,
                 outcome="ok" if tok else "error",
                 **({} if tok else {"error": "castle-unavailable"}))
            return tok, used_dev

    async def _account(self, account, proxy=None):
        async with self._activity(account):
            return await self._account_active(account, proxy)

    async def _account_active(self, account, proxy=None):
        if self._closed:
            raise RuntimeError("browser host is closed")
        life = self._lifecycle(account)
        life.rates.check_admission()
        self._browser_status(account)
        if life.phase in ("closing", "unconfirmed"):
            raise RuntimeError("account browser cleanup is unconfirmed; retry after cleanup")
        if account in self.accounts and life.phase == "resident":
            return self.accounts[account]
        async with life.creation_lock:
            life.rates.check_admission()
            if self._closed:
                raise RuntimeError("browser host is closed")
            if life.phase in ("closing", "unconfirmed"):
                raise RuntimeError("account browser cleanup is unconfirmed; retry after cleanup")
            if account in self.accounts:
                return self.accounts[account]
            life.incarnation += 1
            life.phase = "starting"
            try:
                acc = await self._launch(account, proxy)
            except BaseException:
                if life.phase == "starting":
                    life.phase = "absent"
                raise
            life.phase = "resident"
            return acc

    @asynccontextmanager
    async def _page(self, account, proxy=None):
        page, acc, _wait = await self._acquire(account, proxy)
        try:
            yield page
        finally:
            self._release(acc, page)

    async def _start_driver(self, factory):
        if self._playwright is not None:
            return
        if self._driver_start_task is None:
            async def start():
                self._playwright = await factory().start()
            self._driver_start_task = asyncio.create_task(start())
        task = self._driver_start_task
        try:
            await asyncio.shield(task)
        finally:
            if task.done():
                self._driver_start_task = None

    async def _launch(self, account, proxy=None):
        if self._browser_runtime is None:
            runtime = browser_runtime()
            self._memory_browser = prepared_memory_browser()
            self._memory_addon = prepared_memory_addon()
            self._browser_runtime = runtime
        AsyncNewBrowser, async_playwright = self._browser_runtime
        if proxy:
            self.proxies[account] = proxy
        profile = os.path.join(ROOT, account)
        os.makedirs(profile, exist_ok=True)
        # CONTEXT-PER-DEVICE model (2026-10-11): a NON-persistent browser, and
        # every device gets its own isolated ephemeral context (own storage —
        # verified: ctx A cannot see ctx B's cookies). The old persistent
        # single-context profile forced the __cuid-swap churn that flagged us.
        # Browser launch timeout 30s -> 180s: under heavy machine load (the
        # iMac's chromium build pinned load avg >100), the juggler handshake
        # starves past the default 30s and every launch dies in a retry loop —
        # 180s lets it complete slowly instead of never (2026-10-11 stall).
        kwargs = dict(headless=True, humanize=False, timeout=600_000,
                      firefox_user_prefs={
                          "dom.ipc.processCount": 1,
                          "dom.ipc.processCount.webIsolated": 1,
                          "browser.cache.memory.enable": False,
                          "browser.cache.disk.enable": False,
                          "browser.sessionhistory.max_entries": 2,
                      })
        if self._memory_addon is not None:
            kwargs["addons"] = [self._memory_addon["path"]]
            kwargs["exclude_addons"] = [self._memory_addon["excluded_default"]]
        if self._memory_browser:
            kwargs["executable_path"] = self._memory_browser
            kwargs["firefox_user_prefs"]["framerize.captureResponseBodies"] = False
            # v3 ignores this preference; v4 skips unused debugger upload copies.
            kwargs["firefox_user_prefs"]["framerize.captureRequestBodies"] = False
            # v6 distinguishes completed requests from actual redirect chains.
            kwargs["firefox_user_prefs"]["framerize.reportActualRedirects"] = True
            # v8 also releases native caches during the existing idle GC.
            # This preference defaults off and does not unload documents.
            kwargs["firefox_user_prefs"]["framerize.reclaimGraphicsOnGC"] = (
                os.environ.get("FRAMERIZE_IDLE_CACHE_RECLAIM") == "1")
        # humanize=False (2026-10-04, measured): with True, Camoufox simulates
        # human input TIMING — every mouse.move burst cost ~19-21s inside the
        # mint lock (burst_ms=20,991 of a 21,074ms "mint"). Playwright input is
        # TRUSTED either way (Juggler Input domain), so Castle's listeners
        # still fire; we only drop the deliberate slowness.
        # Per-account egress: Castle binds tokens to egress IP (verified live).
        px = self.proxies.get(account) or os.environ.get("FRAMERIZE_PROXY")
        if px:
            kwargs["proxy"] = proxy_options(px)
        trace(f"{account[:8]}: launching camoufox (profile {profile})")
        t0 = time.monotonic()
        # Isolated browser fixtures peaked near 1 GiB. Reserve startup
        # headroom, not the former 256 MiB estimate below even warm usage.
        # This admission estimate is not a cap on later browser growth.
        async with self._allocation(BROWSER_STARTUP_RESERVE):
            life = self.lifecycles[account]
            life.rates.runtime_starting()
            if self._playwright is None:
                await self._rpc(account, self._start_driver, async_playwright)
            # Each launch still has its own process, profile, configuration,
            # proxy and storage. Only the Playwright transport is shared.
            async def launch_owned():
                created = await AsyncNewBrowser(self._playwright, **kwargs)
                # Retain both the handle and its matching cleanup receipt before
                # returning to a waiter that may already have been cancelled.
                life.browser = created
                life.close_receipt = BrowserCloseReceipt(created)
                return created
            try:
                browser = await self._rpc(account, launch_owned)
            except BaseException:
                # A cancelled handshake may have launched a process without
                # returning its handle. Never turn that uncertainty into room.
                life.phase = "unconfirmed"
                raise
            emit("camoufox.launch", account=account[:8], duration_ms=int((time.monotonic()-t0)*1000), outcome="ok")
            try:
                acc = await self._initialize_account(account, profile, browser)
                return acc
            except BaseException:
                acc = self.accounts.get(account)
                await asyncio.shield(self._start_close(account, life, acc))
                raise

    async def _initialize_account(self, account, profile, browser):
        ctx = await self._rpc(account, browser.new_context)   # ambient context (login flow, heartbeats' home)
        page = await self._rpc(account, ctx.new_page)
        trace(f"{account[:8]}: goto login page")
        await self._rpc(account, page.goto, LOGIN_URL, wait_until="domcontentloaded", timeout=90000)
        trace(f"{account[:8]}: dom loaded, waiting for app boot")
        # Gate on the APP being up (login form renders), NOT on
        # window.castlePublishableAPIKey — Firefox never gets that global.
        try:
            await self._rpc(account, page.wait_for_selector, "input[type=email], input[name=email], form", timeout=30000)
            trace(f"{account[:8]}: page ready")
        except Exception as e:
            trace(f"{account[:8]}: app-boot wait gave up ({type(e).__name__}) — continuing anyway")
        await self._rpc(account, page.wait_for_timeout, 1500)
        # Warm the Castle wrapper: the FIRST m.a() pays the v3 attempt —
        # measured 27-81s in Camoufox/Firefox (the v3 SDK's token-generation
        # can never complete in this engine; verified with module pre-warming
        # on 2026-10-02). That failure latches the wrapper's internal v3-off
        # flag, so every LATER m.a() on this page goes straight to the v2
        # fallback (~1s — Framer's own designed fallback, accepted server-side;
        # Firefox framer.com visitors get v2 too). Pay it ONCE here, hidden in
        # page setup — never inside a real mint (login + every freechat chunk).
        trace(f"{account[:8]}: warming castle wrapper (one v3-race ~6s, latches v3-off)")
        t0 = time.monotonic()
        try:
            warm_ok = await self._rpc(account, page.evaluate, WARM_JS)
            emit("castle.warm", account=account[:8], latched=bool(warm_ok),
                 duration_ms=int((time.monotonic()-t0)*1000), outcome="ok")
            trace(f"{account[:8]}: castle warm {'latched — mints now ~1s' if warm_ok else 'no token yet, latch set anyway'}")
        except Exception as e:
            trace(f"{account[:8]}: castle warm failed ({type(e).__name__}) — first real mint will pay it instead")
        acc = Account(None, ctx, page, self.lifecycles[account])
        acc.browser = browser
        acc.editor_file = os.path.join(profile, "framerize-editor.json")
        try:
            if os.path.exists(acc.editor_file):
                acc.editor_url = json.loads(Path(acc.editor_file).read_text()).get("editor_url")
                if acc.editor_url:
                    trace(f"{account[:8]}: editor context restored ({acc.editor_url.split('/')[-1][:12]}...)")
        except Exception:
            pass
        self.accounts[account] = acc
        self._track_task(acc, self._human_loop(account, page), page=page, idle_loop=True)
        # Device pool (the concurrency goose): persisted uuids per account,
        # castle-module instances warmed concurrently in the background.
        try:
            acc.lifecycle.rates.ensure_devices(DEVICE_POOL)
            # Device contexts are created LAZILY on first use (each is a real
            # browser context — eager-warming N of them at boot is the slow
            # part). devpool_ready kept for compat with the token path.
            acc.devpool_ready = True
            self._track_task(acc, self._heartbeat_loop(acc, account), idle_loop=True)
        except Exception as e:
            trace(f"{account[:8]}: device pool init failed ({type(e).__name__})")
            raise
        # Device requests use their own page. Allocate the ambient mint page
        # only when the legacy mint path actually needs it.
        return acc

    async def _heartbeat_loop(self, acc, account):
        """Keep every pool device on a steady learned-as-normal cadence: one
        tiny free-route AI request per HEARTBEAT_S, cycling all devices equally
        (each device ~DEVICE_POOL x HEARTBEAT_S between beats). Waits for the
        pool to warm, skips cooling devices, uses the mint page so production
        pool pages are never blocked, and tolerates every failure silently —
        the heartbeat is pure background training, never a request source."""
        import random as _rnd
        await asyncio.sleep(45)  # let the account boot + pool warm first
        hb_idx = 0
        hb_interval = HEARTBEAT_S  # adaptive: hb 403s double it (curse decay), 200s recover
        while self.accounts.get(account) is acc and not acc.closing:
            try:
                # Operator override: ~/.framerize/heartbeat-skip.json = ["alias", ...]
                # or ["*"] — hot-reloaded each cycle (2026-10-08: an escalated
                # account's heartbeat keeps its account-level activity score
                # hot forever; skip it and let the score decay).
                try:
                    skip = json.loads(Path("~/.framerize/heartbeat-skip.json").expanduser().read_text())
                    if "*" in skip or account in skip or getattr(self, "aliases", {}).get(account) in skip:
                        await asyncio.sleep(60)
                        continue
                except Exception:
                    pass
                await asyncio.sleep(hb_interval * (0.75 + _rnd.random() * 0.5))
                async with self._activity(account):
                    if self.accounts.get(account) is not acc or acc.closing:
                        return
                    if not acc.devices or not acc.devpool_ready or not acc.project:
                        continue
                    now = time.monotonic()
                    dev = None
                    for _ in range(len(acc.devices)):
                        d = acc.devices[hb_idx % len(acc.devices)]
                        hb_idx += 1
                        if acc.dev_cooling.get(d, 0) > now:
                            continue
                        dev = d
                        break
                    if dev is None:
                        continue
                    # Fire in the DEVICE'S OWN context (2026-10-11 audit): the old
                    # path fired on the ambient mint page, whose __cuid got synced
                    # to a different device every beat — reintroducing the exact
                    # churn this architecture exists to hide, concentrated in one
                    # page's storage for the SDK to see.
                    try:
                        page = (await self._dev_page(acc, account, dev)).page
                    except Exception:
                        continue
                    tid, tok, used_dev = await self._token_acquire(acc, page, account, dev)
                    if not tok:
                        continue
                    self._note_request(acc, used_dev)
                    body = {
                        "projectId": acc.project, "featureCategory": "translation",
                        "featureName": "translation", "step": "translate",
                        "sessionId": "hb-" + uuid.uuid4().hex[:6], "stepIndex": 0,
                        "turnId": uuid.uuid4().hex, "userPrompt": "ok",
                        "request": {"model": "google/gemini-2.5-flash",
                                    "messages": [{"role": "user", "content": "ok"}],
                                    "max_tokens": 8, "stream": False},
                    }
                    job = "hb-" + uuid.uuid4().hex
                    try:
                        r = await self._run_job(page, "/ai/v3/messages/", body, tok,
                                                acc.bearer, job, used_dev, 30)
                        st = r.get("status")
                    finally:
                        self._token_release(acc, tid)
                    if st == 403:
                        self._cool_device(acc, used_dev, account)
                        # The heartbeat ITSELF got velocity-banned: this account's
                        # score is so escalated that even background beats trip it.
                        # Back off hard — every further beat just keeps the score
                        # hot. Doubling per ban (cap ~2h); recovery at half speed.
                        hb_interval = min(hb_interval * 2, 7200)
                        trace(f"{account[:8]}: heartbeat banned — interval now {int(hb_interval)}s")
                    else:
                        hb_interval = max(HEARTBEAT_S, hb_interval * 0.9)
                        if st is not None and st != 200:
                            trace(f"{account[:8]}: heartbeat got {st} (device {str(used_dev)[:8]})")
            except asyncio.CancelledError:
                raise
            except Exception:
                await asyncio.sleep(30)

    def _ctx(self, account):
        return self.accounts[account].ctx

    def _account_for_page(self, page):
        owners = [account for account, acc in self.accounts.items()
                  if page is acc.page or page is acc.mint_page
                  or any(page is device.page for device in acc.dev_objs.values())]
        if len(owners) != 1:
            raise RuntimeError("browser page account ownership is unconfirmed")
        return owners[0]

    @staticmethod
    def _take_ai_post_failure_facts(result, job):
        """Strip private diagnostics; a received head does not complete a response."""
        phase = result.pop("failure_phase", None)
        status = result.pop("head_status", None)
        if type(result.get("status")) is not int or result["status"] != -1:
            return {}
        if phase == "fetch" and status is None:
            return {"failure_phase": "fetch", "head_status": None, "job": job}
        if (phase == "body" and type(status) is int
                and (status == 0 or 100 <= status <= 599)):
            return {"failure_phase": "body", "head_status": status, "job": job}
        return {}

    async def _run_job(self, page, path, body, token, bearer, job, dev, timeout):
        account = self._account_for_page(page)
        async with self._activity(account):
            return await self._run_job_active(account, page, path, body, token,
                                               bearer, job, dev, timeout)

    async def _run_job_active(self, account, page, path, body, token, bearer, job, dev, timeout):
        deadline = time.monotonic() + timeout
        completed = False
        try:
            await asyncio.wait_for(
                self._rpc(account, page.evaluate, AI_START_JS, [path, body, token, bearer, job, dev]),
                min(30, timeout),
            )
            emit("ai.post.started", job=job, path=path)
            async def poll(acknowledged):
                return await self._rpc(account, page.evaluate, AI_POLL_JS,
                                       [job, acknowledged])
            try:
                result = await receive_response(poll, deadline)
            except ResponseDeadline:
                return {"status": -1, "usage": None, "body": f"sidecar poll timeout ({timeout:g}s)"}
            completed = True
            return result
        finally:
            try:
                if not completed:
                    self.lifecycles[account].rates.mark_uncertain()
            finally:
                # Request cancellation even if recording uncertainty fails.
                try:
                    await asyncio.wait_for(self._rpc(account, page.evaluate, AI_CANCEL_JS, job), 2)
                except (Exception, asyncio.CancelledError):
                    pass

    async def handle(self, req):
        if self._closed:
            raise RuntimeError("browser host is shutting down")
        if req.get("cmd") == "history_status":
            accounts = req.get("accounts")
            if (type(accounts) is not list or len(accounts) > 64
                    or any(type(account) is not str for account in accounts)
                    or len(set(accounts)) != len(accounts)):
                raise ValueError("invalid history readiness accounts")
            for account in accounts:
                self._lifecycle(account).rates.check_admission()
            return {"ready": True, "accounts": len(accounts)}
        account = req.get("account", "")
        # Absent status/registration are synchronous and cannot race admission.
        # Preserve their existing no-browser/no-lifecycle-allocation behavior.
        if (account not in self.lifecycles
                and req.get("cmd") in ("register_proxy", "browser_status")):
            return await self._handle_account_command(req)
        async with self._activity(account):
            return await self._handle_account_command(req)

    async def _handle_account_command(self, req):
        if req.get("cmd") == "register_proxy":
            return self._register_proxy(req)
        if req.get("cmd") == "browser_status":
            return self._browser_status(req["account"])
        if req.get("cmd") == "close_idle":
            return await self._close_idle(req)
        if req.get("cmd") in ("close", "rotate"):
            self._check_proxy_revision(req)
            account = req.get("account", "")
            if not await self._close_account(account):
                raise RuntimeError("account is busy or browser cleanup is unconfirmed")
            if req["cmd"] == "rotate":
                shutil.rmtree(os.path.join(ROOT, account), ignore_errors=True)
            return True
        if req.get("cmd") not in ("begin", "complete", "mint", "ai_post", "set_cookie", "eval"):
            raise RuntimeError("unknown cmd %s" % req.get("cmd"))
        self._adopt_proxy_route(req)
        async with self._use_account(req.get("account", ""), req.get("proxy")):
            return await self._handle(req)

    async def _handle(self, req):
        cmd = req.get("cmd")
        account = req.get("account", "")
        if cmd == "begin":
            async with self._page(account, req.get("proxy")) as page:
                trace(f"{account[:8]}: mint+signin eval start")
                t0 = time.monotonic()
                r = await self._rpc(account, page.evaluate, post_js("/auth/v4/signin", {"email": req["email"], "origin": "web"}))
                trace(f"{account[:8]}: signin -> status {r['status']}")
                ok = 200 <= r["status"] < 300
                emit("auth.signin", account=account[:8], status=r["status"],
                     duration_ms=int((time.monotonic()-t0)*1000), outcome="ok" if ok else "error",
                     **({} if ok else {"error": r["body"][:300]}))
                if not ok:
                    raise RuntimeError("signin %s: %s" % (r["status"], r["body"][:400]))
                return {"status": r["status"]}
        if cmd == "complete":
            async with self._page(account) as page:
                acc = self.accounts[account]
                r = await self._rpc(account, page.evaluate, post_js("/auth/v4/continue", {"token": req["token"]}))
                if not (200 <= r["status"] < 300):
                    raise RuntimeError("continue %s: %s" % (r["status"], r["body"][:400]))
                me = await self._rpc(account, page.evaluate, """(async () => {
                  const r = await fetch("%s/web/users/me", { headers: { "x-requested-by": "Framer" }, credentials: "include" });
                  return { status: r.status, body: await r.text() };
                })()""" % API)
                if not (200 <= me["status"] < 300):
                    raise RuntimeError("me %s: %s" % (me["status"], me["body"][:400]))
                cookies = await self._rpc(account, self._ctx(account).cookies, "https://framer.com")
                session = next((c["value"] for c in cookies if c["name"] == "session"), None)
                if not session:
                    emit("auth.complete", account=account[:8], level="error", outcome="error", error="no session cookie after continue")
                    raise RuntimeError("no session cookie after continue")
                emit("auth.complete", account=account[:8], outcome="ok")
                me_value = json.loads(me["body"])
                return {"cookie_header": "session=" + session, "me": me_value}
        if cmd == "mint":
            page, acc, queue_wait = await self._acquire(account)
            try:
                await self._ensure_editor(acc, page, account)
                tok, _dev = await self._mint(acc, page, account)
            finally:
                self._release(acc, page)
            if not tok:
                raise RuntimeError("castle-unavailable")
            return tok
        if cmd == "ai_post":
            acc = await self._account(account)
            page = acc.page  # ambient page (fallback only)
            job = None
            queue_wait = 0.0
            t0 = time.monotonic()
            pid = (req.get("body") or {}).get("projectId")
            if pid:
                self._set_editor(acc, account, pid)
            try:
                # Mint under the per-account gate, THEN fetch unguarded: the
                # mint is the risk-engine-sensitive burst, the generation is
                # just a long I/O await that parallelizes freely.
                # VELOCITY GOOSE: pick a device by budget, fire, and if the
                # device is velocity-banned (403) cool it and INSTANTLY retry
                # on the next device — a per-device ban never reaches callers.
                attempts = 1 + min(2, len(acc.devices or [None]))
                defer_retries = req.get("defer_retries") is True
                first = req.get("attempt_index", 0) if defer_retries else 0
                if type(first) is not int or not 0 <= first < attempts:
                    raise ValueError("invalid browser attempt index")
                r = None
                for attempt in range(first, first + 1 if defer_retries else attempts):
                    dev = self._pick_device_sticky(acc) or self._pick_device(acc, account)
                    if dev is None and acc.devices:
                        # Everything cooling/over-budget: WAIT for the soonest
                        # recovery instead of poking a banned device (the poke
                        # re-bans for the full 600s — the cascade). Bounded:
                        # long waits surface as velocity-paused so the chain
                        # backs off honestly instead of minting into a ban.
                        wait = self._device_ready_in(acc)
                        if wait is not None and wait <= 60.0:
                            trace(f"{account[:8]}: all devices busy/cooling — waiting {wait:.0f}s for recovery (no re-poke)")
                            await asyncio.sleep(wait + 0.25)
                            dev = self._pick_device_sticky(acc) or self._pick_device(acc, account)
                        if dev is None:
                            w = self._device_ready_in(acc) or 0
                            r = {"status": 0, "usage": None,
                                 "body": f"velocity-paused: every device is cooling; retry in {int(w)}s"}
                            break
                    fire_page = page
                    if dev is not None and acc.browser is not None:
                        try:
                            fire_page = (await self._dev_page(acc, account, dev)).page
                        except Exception:
                            fire_page = page
                    if fire_page is page:
                        await self._ensure_editor(acc, page, account)
                    tid, tok, used_dev = await self._token_acquire(acc, fire_page, account, dev)
                    if not tok:
                        r = {"status": 0, "usage": None, "body": "castle-unavailable"}
                        break
                    try:
                        self._note_request(acc, used_dev)
                        fetch_t0 = time.monotonic()
                        job = "job-" + uuid.uuid4().hex
                        acc.project = req["body"].get("projectId") or acc.project
                        acc.bearer = req.get("bearer") or acc.bearer
                        # Deadline scales with the chunk's expected duration:
                        # a 4300-token chunk at collapse speed (~11 tok/s)
                        # legitimately needs ~390s — the flat 310s raced it and
                        # three chains died mid-generation (2026-10-05 audit).
                        # Read max_output_tokens too: the nativechain Responses
                        # payload never sets max_tokens, so the old default-600
                        # math was a lie for Codex.
                        est_s = 60.0
                        try:
                            rb = req["body"].get("request", {}) or {}
                            tok_n = rb.get("max_tokens") or rb.get("max_output_tokens") or 600
                            est_s = max(60.0, float(tok_n) / 10.0 * 3.0)
                        except Exception:
                            pass
                        r = await self._run_job(
                            fire_page, req["path"], req["body"], tok,
                            req.get("bearer"), job, used_dev, min(180.0, est_s),
                        )
                        r["_fetch_ms"] = int((time.monotonic() - fetch_t0) * 1000)
                    finally:
                        # release only after the response fully arrived — the
                        # token's request is then complete, so a later use is
                        # the proven-safe SEQUENTIAL case (never concurrent).
                        self._token_release(acc, tid)
                    # Velocity 403s are FAST (~50-150ms measured); the
                    # idle-window kill is also a 403 but arrives after 8+s of
                    # silence — retrying that on a fresh device would just die
                    # again and wrongly cool the device. Only rotate on fast 403s.
                    if r["status"] == 403 and r.get("_fetch_ms", 9999) < 4000 and attempt + 1 < attempts:
                        # TTL vs velocity (2026-10-05): TTL enforcement (~600s) makes
                        # OLD tokens 403 — that is EXPIRY, not a velocity ban, and
                        # cooling the device over it drains the pool for nothing
                        # (the cascade that killed chains today). Only a fresh
                        # token's 403 means velocity.
                        tok_age = time.monotonic() - acc.token_pool.get(tid, {}).get("born", acc.token_pool.get(tid, {}).get("ts", time.monotonic())) if tid else 0
                        if tid and tok_age > self.TOKEN_TTL_S * 0.8:
                            acc.token_pool.pop(tid, None)  # expired — refresh, same device
                            trace(f"{account[:8]}: token 403 at age {tok_age:.0f}s — TTL expiry, refreshing (device NOT cooled)")
                            if defer_retries:
                                r["_retry_device"] = True
                                break
                            continue
                        # velocity ban is per-device (proven): cool this one,
                        # rotate, retry — invisible to the caller.
                        self._cool_device(acc, used_dev, account)
                        if defer_retries:
                            r["_retry_device"] = True
                            break
                        continue
                    break
            finally:
                pass  # device-context pages are long-lived; nothing to release
            emit("ai.post", account=account[:8], path=req["path"], status=r["status"],
                 duration_ms=int((time.monotonic()-t0)*1000),
                 queue_wait_ms=int(queue_wait*1000),
                 outcome="ok" if 200 <= r["status"] < 300 else "error",
                 **self._take_ai_post_failure_facts(r, job))
            # queue_wait_ms lets the caller separate "we waited for a page" from
            # "the upstream is slow" — they are identical in wall-clock and the
            # chunk sizer must not conflate them.
            r["queue_wait_ms"] = int(queue_wait * 1000)
            return r
        if cmd == "set_cookie":
            # Context-level op — do NOT acquire a pool page: at cap that's a
            # self-deadlock (every ai_post flow starts with set_cookie, so
            # set_cookie waiting for a free page starves behind the very
            # requests it unblocks; measured: 300s timeouts across the board).
            acc = await self._account(account)
            return await self._rpc(account, set_account_cookie,
                                   self, account, acc, req["name"], req["value"])
        if cmd == "eval":
            async with self._page(account) as page:
                return await self._rpc(account, page.evaluate, req["expr"])
        raise RuntimeError("unknown cmd %s" % cmd)

    async def _serve(self, req, writer):
        """Run one command and write its reply. Replies are id-matched, so they
        may be emitted out of order — the Rust side routes by id."""
        request_id = req.get("id")
        try:
            result = await self.handle(req)
            out = {"id": request_id, "ok": True, "result": result}
        except Exception as e:
            if req.get("cmd") == "history_status":
                from framerize_accounts.diagnostics import describe, report
                report(e, 'history.readiness.failed')
                emit('history.readiness.failed', level='error', **describe(e))
            out = {"id": request_id, "ok": False, "error": str(e)[:800]}
        finally:
            # The handler owns input only until it finishes. A slow reply
            # consumer must not keep a completed request body alive.
            req = None
        # The writer owns framing and backpressure. A slow reader must not
        # block browser jobs, cancellations or the memory monitor.
        await writer.send(out)

    async def loop(self):
        trace("sidecar ready (async)")
        loop = asyncio.get_running_loop()
        writer = AsyncReplyWriter(sys.stdout)
        # Include pending commands and detached frame writers in the idle gate.
        self._reply_writer = writer
        tasks = self._command_tasks = {}
        monitor = asyncio.create_task(self._monitor())
        graceful = False
        try:
            while True:
                line = await loop.run_in_executor(None, sys.stdin.readline)
                try:
                    if not line:
                        break
                    if not line.strip():
                        continue
                    try:
                        try:
                            req = json.loads(line)
                        finally:
                            line = None
                        if not isinstance(req, dict):
                            raise ValueError("command must be an object")
                    except Exception as e:
                        await writer.send({"id": None, "ok": False, "error": str(e)[:800]})
                        continue
                    if req == {"cmd": "shutdown"}:
                        graceful = True
                        break
                    if req.get("cmd") == "cancel":
                        task = tasks.get(req.get("request_id"))
                        if task:
                            task.cancel()
                        continue
                    request_id = req.get("id")
                    if request_id in tasks or len(tasks) >= 512:
                        await writer.send({"id": request_id, "ok": False,
                                           "error": "sidecar busy or duplicate request id"})
                        continue
                    task = asyncio.create_task(self._serve(req, writer))
                    tasks[request_id] = task
                    task.add_done_callback(lambda done, key=request_id: tasks.pop(key, None))
                finally:
                    # The next read can wait indefinitely. Only active tasks
                    # should retain decoded input, never the reader itself.
                    line = req = task = None
        finally:
            # Explicit shutdown drains accepted commands. EOF stops follow-up
            # work, while close still joins the raw RPCs owned by those commands.
            monitor.cancel()
            pending = tuple(tasks.values())
            if not graceful:
                for task in pending:
                    task.cancel()
            await asyncio.gather(monitor, *pending, return_exceptions=True)
            await writer.close()
            self._reply_writer = None
            await self.close()
