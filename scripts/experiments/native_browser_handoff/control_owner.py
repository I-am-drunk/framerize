"""Dedicated experimental caller. No command-line or live-service wiring.

One control owns its selected page through observation and consumer settlement.
An uncertain operation fences admission permanently. Physical close remains
available to the host's own shutdown tasks after the operation has joined.
"""
import asyncio
from contextlib import asynccontextmanager
from dataclasses import dataclass
import time

from framerize_host.app import Host
from control_mint import ObservationRefused, _validate, observe


@dataclass(frozen=True, repr=False)
class ControlIdentity:
    account: str
    host_generation: str
    incarnation: int
    proxy_revision: int


@dataclass(frozen=True, repr=False)
class Consumed:
    value: object
    cleanup_confirmed: bool


def refuse():
    raise ObservationRefused()


class _Binding:
    def __init__(self, host, identity, expected):
        if (type(identity) is not ControlIdentity
                or type(identity.account) is not str or not identity.account
                or len(identity.account) > 256
                or type(identity.host_generation) is not str
                or type(identity.incarnation) is not int or identity.incarnation <= 0
                or type(identity.proxy_revision) is not int
                or not 0 <= identity.proxy_revision < 2 ** 64):
            refuse()
        self.host, self.identity, self.expected = host, identity, expected
        self.accounts, self.lifecycles = host.accounts, host.lifecycles
        self.acc = self.accounts.get(identity.account)
        self.life = self.lifecycles.get(identity.account)
        if self.acc is None or self.life is None:
            refuse()
        self.device = self.acc.dev_objs.get(expected.device)
        if self.device is None:
            refuse()
        self.page, self.context = self.device.page, self.device.ctx
        self.browser, self.activity = self.acc.browser, self.life.activity
        self.proxy = host.proxies.get(identity.account)
        if not self.current():
            refuse()
        if (self.life.active_commands or self.acc.active_requests
                or self.acc.borrowed or self.activity.activities or self.activity.pending
                or host._command_tasks or any(not task.done() for task in self.acc.tasks)):
            refuse()

    def current(self):
        h, i, e = self.host, self.identity, self.expected
        try:
            return (not h._closed and not h._control_cancelled.is_set()
                and h.generation == i.host_generation
                and h.accounts is self.accounts and h.lifecycles is self.lifecycles
                and set(self.accounts) == {i.account} and set(self.lifecycles) == {i.account}
                and self.accounts.get(i.account) is self.acc
                and self.lifecycles.get(i.account) is self.life
                and self.acc.lifecycle is self.life
                and self.life.incarnation == i.incarnation and self.life.phase == "resident"
                and self.life.close_task is None and not self.acc.closing
                and self.life.activity is self.activity
                and h.proxy_revisions.get(i.account, 0) == i.proxy_revision
                and h.proxies.get(i.account) == self.proxy
                and self.acc.session_cookie == ("session", e.session)
                and self.acc.dev_objs.get(e.device) is self.device
                and self.device.id == e.device and self.device.page is self.page
                and self.device.ctx is self.context and self.page.context is self.context
                and not self.page.is_closed() and self.page.url == e.page_url
                and self.context.browser is self.browser
                and self.acc.browser is self.browser and self.life.browser is self.browser
                and self.browser.is_connected() is True)
        except Exception:
            return False


class ControlHost(Host):
    def __init__(self, rate_journal=None):
        super().__init__(rate_journal)
        self._control_task = None
        self._control_busy = self._control_used = self._control_fenced = False
        self._control_cancelled = asyncio.Event()

    def _cleanup_owner(self):
        task = asyncio.current_task()
        return (self._closed and self._close_task is not None and task is not None
            and (task is self._close_task
                 or any(task is life.close_task for life in self.lifecycles.values())))

    def _admit_ordinary(self):
        if (self._closed or self._control_busy or self._control_fenced) and not self._cleanup_owner():
            refuse()

    @asynccontextmanager
    async def _activity(self, account):
        self._admit_ordinary()
        async with super()._activity(account):
            yield

    async def _rpc(self, account, call, *args, **kwargs):
        self._admit_ordinary()
        return await super()._rpc(account, call, *args, **kwargs)

    async def handle(self, req):
        self._admit_ordinary()
        return await super().handle(req)

    async def run_control(self, identity, expected, consume, deadline):
        """The consumer must return only after its actual request has settled."""
        try:
            self._admit_ordinary()
            if self._control_used or not callable(consume):
                refuse()
            _validate(expected, deadline)
            binding = _Binding(self, identity, expected)
        except Exception:
            raise ObservationRefused() from None
        self._control_used = self._control_busy = True
        task = self._control_task = asyncio.create_task(
            self._drive_control(binding, expected, consume, deadline))
        task.add_done_callback(lambda done: None if done.cancelled() else done.exception())
        try:
            return await asyncio.shield(task)
        except asyncio.CancelledError:
            self._control_fenced = True
            self._control_cancelled.set()
            raise

    async def _control_rpc(self, binding, deadline, call, *args):
        if not binding.current() or time.monotonic() >= deadline:
            refuse()
        result = await asyncio.wait_for(
            super()._rpc(binding.identity.account, call, *args),
            max(0, deadline - time.monotonic()))
        if not binding.current() or time.monotonic() >= deadline:
            refuse()
        return result

    async def _drive_control(self, binding, expected, consume, deadline):
        consumer = outcome = None
        failed = False
        try:
            async def rpc(call, *args):
                return await self._control_rpc(binding, deadline, call, *args)
            minted = await observe(binding.page, expected, rpc, binding.current, deadline)
            if not binding.current() or time.monotonic() >= deadline:
                refuse()
            consumer = asyncio.create_task(consume(minted, self._control_cancelled, deadline))
            outcome = await asyncio.wait_for(asyncio.shield(consumer),
                max(0, deadline - time.monotonic()))
            if (type(outcome) is not Consumed or outcome.cleanup_confirmed is not True
                    or not binding.current() or time.monotonic() >= deadline):
                refuse()
        except (Exception, asyncio.CancelledError):
            failed = self._control_fenced = True
            self._control_cancelled.set()
        finally:
            jobs = ([] if consumer is None else [consumer]) + list(binding.activity.pending)
            joined = await asyncio.gather(*(asyncio.shield(job) for job in jobs),
                return_exceptions=True)
            if any(isinstance(value, BaseException) for value in joined):
                failed = self._control_fenced = True
            self._control_busy = False
        if failed or not binding.current():
            self._control_fenced = True
            refuse()
        return outcome

    async def _close_host_active(self):
        self._control_cancelled.set()
        if self._control_task is not None:
            await asyncio.gather(asyncio.shield(self._control_task), return_exceptions=True)
        return await super()._close_host_active()
