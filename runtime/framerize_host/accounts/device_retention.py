"""Release idle device contexts without changing their durable identities."""
import asyncio

from .. import events
from .page_use import page_use


def _idle(host, account, acc, locked=None, maintenance=None, rpc=None):
    life = acc.lifecycle
    activities = life.activity.activities
    expected = 0 if maintenance is None else 2
    return (not host._closed and host.accounts.get(account) is acc
        and host.lifecycles.get(account) is life and life.browser is acc.browser
        and life.phase == "resident" and not acc.closing
        and life.close_task is None and not life.active_commands
        and not acc.active_requests and not acc.borrowed
        and not acc.mint_lock.locked() and not acc.cookie_lock.locked()
        and not acc.lock.locked() and not life.creation_lock.locked()
        and len(activities) == expected
        and (maintenance is None or maintenance in activities)
        and life.activity.pending == (set() if rpc is None else {rpc})
        and not any(d is not locked and d.lock.locked()
                    for d in acc.dev_objs.values())
        and not any(t.get("in_use") for t in acc.token_pool.values()))


def _same_device(host, account, acc, device, context, page):
    return (host.accounts.get(account) is acc
        and host.lifecycles.get(account) is acc.lifecycle
        and acc.dev_objs.get(device.id) is device
        and device.ctx is context and device.page is page
        and acc.lifecycle.browser is acc.browser
        and acc.lifecycle.phase == "resident" and not acc.closing)


async def _close_idle(host, account, acc, device):
    if device.lock.locked():
        return False
    async with device.lock:
        if not _idle(host, account, acc, locked=device):
            return False
        context, page = device.ctx, device.page
        if context is None or page is None or device.id == acc.primary_dev:
            return False
        with acc.lifecycle.activity.activity() as maintenance:
            async def close_owned():
                # Host._rpc contributes the second activity and owns this task.
                if (not _idle(host, account, acc, device, maintenance,
                              asyncio.current_task())
                        or not _same_device(host, account, acc, device, context, page)
                        or device.id == acc.primary_dev):
                    return False
                try:
                    await context.close()
                    if (not _same_device(host, account, acc, device, context, page)
                            or not acc.browser.is_connected()
                            or context in acc.browser.contexts
                            or not page.is_closed()):
                        raise RuntimeError("device context close is unconfirmed")
                except BaseException:
                    acc.closing = True
                    acc.lifecycle.phase = "unconfirmed"
                    events.emit("browser.cleanup", level="warn", outcome="unconfirmed",
                                error="idle-device-context-close-failed")
                    raise
                device.ctx, device.page, device.warm = None, None, False
                return True

            async with page_use(account):
                try:
                    return await host._rpc(account, close_owned)
                except Exception:
                    return False


async def trim_idle_device_contexts(host, account, acc, keep=2):
    """Close at most two excess contexts; keep the primary and recent devices."""
    if type(keep) is not int or keep < 1:
        raise ValueError("device context retention must be positive")
    closed = 0
    for _ in range(2):
        if not _idle(host, account, acc):
            break
        resident = [d for d in acc.dev_objs.values()
                    if d.ctx is not None and d.page is not None]
        if len(resident) <= keep:
            break
        recent = sorted(resident, key=lambda d: (d.id == acc.primary_dev,
                                                d.last_used), reverse=True)
        victim = recent[-1]
        if not await _close_idle(host, account, acc, victim):
            break
        closed += 1
    return closed
