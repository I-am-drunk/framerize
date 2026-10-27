"""Explicit single-page preparation. No entry point or live egress claim.

Native ownership supplies admission and the existing identity. The temporary
host uses in-memory lifecycle state and never starts ordinary host commands.
"""
import asyncio
from contextlib import asynccontextmanager
import json
import os
import time
import uuid

from framerize_host.app import Host
from framerize_host.accounts.state import Account, Device
from control_mint import ObservationRefused, _validate
from control_owner import ControlHost, ControlIdentity, _Binding, refuse


class PreparedControlHost(ControlHost):
    def __init__(self):
        super().__init__(rate_journal=None)
        self._preparation_task = None
        self._preparation_stopped = asyncio.Event()
        self._prepared_identity = None
        self._preparation_life = None

    async def handle(self, req):
        refuse()

    async def prepare(self, account, expected, *, proxy, proxy_revision, deadline):
        try:
            _validate(expected, deadline)
            if (type(account) is not str or str(uuid.UUID(account)) != account
                    or type(proxy_revision) is not int
                    or not 0 <= proxy_revision < 2 ** 64
                    or (proxy is not None and (type(proxy) is not str
                        or not proxy or len(proxy.encode()) > 16384))
                    or (proxy is None and os.environ.get("FRAMERIZE_PROXY"))
                    or self._preparation_task is not None or self._closed
                    or self.accounts or self.lifecycles or self._command_tasks):
                refuse()
        except Exception:
            raise ObservationRefused() from None

        self._account_id, self._expected = account, expected
        self._proxy, self._proxy_revision = proxy, proxy_revision
        self._preparation_deadline = deadline
        self._preparation_generation = self.generation
        self.proxy_revisions[account] = proxy_revision
        if proxy is not None:
            self.proxies[account] = proxy
        task = self._preparation_task = asyncio.create_task(self._prepare_owned())
        try:
            identity = await asyncio.wait_for(asyncio.shield(task),
                max(0, deadline - time.monotonic()))
            if time.monotonic() >= deadline:
                refuse()
            _Binding(self, identity, expected)
            return identity
        except asyncio.CancelledError:
            await self._join_shutdown()
            raise
        except Exception:
            await self._join_shutdown()
            raise ObservationRefused() from None

    async def _prepare_owned(self):
        account = self._account_id
        self._preparation_life = self._lifecycle(account)
        await Host._account(self, account, self._proxy)
        self._check_preparation(account)
        identity = ControlIdentity(account, self.generation,
            self._preparation_life.incarnation, self._proxy_revision)
        _Binding(self, identity, self._expected)
        self._prepared_identity = identity
        return identity

    def _check_preparation(self, account):
        if (asyncio.current_task() is not self._preparation_task
                or self._preparation_stopped.is_set() or self._closed
                or time.monotonic() >= self._preparation_deadline
                or account != self._account_id
                or self.generation != self._preparation_generation
                or self.lifecycles.get(account) is not self._preparation_life
                or set(self.lifecycles) != {account}
                or not set(self.accounts) <= {account}
                or self.proxy_revisions.get(account) != self._proxy_revision
                or self.proxies.get(account) != self._proxy
                or (self._proxy is None and os.environ.get("FRAMERIZE_PROXY"))):
            refuse()

    def _preparation_cleanup(self):
        task = asyncio.current_task()
        return self._cleanup_owner() or (
            self._preparation_task is not None
            and not self._preparation_task.done()
            and task is not None
            and any(task is life.close_task for life in self.lifecycles.values()))

    async def _rpc(self, account, call, *args, **kwargs):
        if self._preparation_cleanup():
            return await Host._rpc(self, account, call, *args, **kwargs)
        self._check_preparation(account)
        result = await Host._rpc(self, account, call, *args, **kwargs)
        self._check_preparation(account)
        return result

    @asynccontextmanager
    async def _activity(self, account):
        if not self._preparation_cleanup():
            self._check_preparation(account)
        async with Host._activity(self, account):
            yield

    async def _initialize_account(self, account, profile, browser):
        expected = self._expected
        ctx = await self._rpc(account, browser.new_context)
        await self._rpc(account, ctx.add_init_script,
            "try { localStorage.setItem('__cuid', "
            + json.dumps(expected.device) + "); } catch(e) {}")
        cookies = [dict(name=name, value=value, domain=".framer.com", path="/",
            secure=True, httpOnly=name == "session", sameSite="None")
            for name, value in (("session", expected.session), ("__cuid", expected.device))]
        await self._rpc(account, ctx.add_cookies, cookies)
        page = await self._rpc(account, ctx.new_page)
        acc = Account(None, ctx, page, self._preparation_life)
        acc.pool.get_nowait()
        acc.size = 0
        acc.browser, acc.session_cookie = browser, ("session", expected.session)
        acc.editor_url = expected.page_url
        device = Device(expected.device)
        device.ctx, device.page = ctx, page
        acc.dev_objs[expected.device] = device
        self.accounts[account] = acc
        await self._rpc(account, page.goto, expected.page_url,
            wait_until="domcontentloaded",
            timeout=max(1, min(90000, int((self._preparation_deadline - time.monotonic()) * 1000))))
        return acc

    async def run_control(self, identity, expected, consume, deadline):
        if (self._prepared_identity is None or identity != self._prepared_identity
                or expected != self._expected):
            refuse()
        return await super().run_control(identity, expected, consume, deadline)

    async def close(self):
        self._preparation_stopped.set()
        self._control_cancelled.set()
        return await super().close()

    async def _close_host_active(self):
        if self._preparation_task is not None:
            await asyncio.gather(asyncio.shield(self._preparation_task),
                return_exceptions=True)
        return await super()._close_host_active()

    async def _join_shutdown(self):
        self._preparation_stopped.set()
        self._control_fenced = True
        closing = asyncio.create_task(self.close())
        # A second cancellation cannot abandon a late-created browser or page.
        # The outer process owner supplies the bound for a permanently stuck RPC.
        while not closing.done():
            try:
                await asyncio.shield(closing)
            except asyncio.CancelledError:
                continue
            except Exception:
                break
        if not closing.cancelled():
            closing.exception()
