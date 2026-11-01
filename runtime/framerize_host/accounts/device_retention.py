"""Release idle device contexts without changing their durable identities."""
import asyncio

from .. import events
from .page_use import page_use

CLOSE_TIMEOUT = 10.0


def _idle(host, account, acc, locked=None, maintenance=None, rpc=None,
          allow_busy=False):
    life = acc.lifecycle
    if allow_busy:
        # Every request holds its device until all dispatched RPCs settle.
        # Other devices may remain busy while this unused context closes.
        return (locked is not None and not locked.users
            and not host._closed and host.accounts.get(account) is acc
            and host.lifecycles.get(account) is life
            and life.browser is acc.browser and life.phase == "resident"
            and not acc.closing and life.close_task is None
            and not acc.cookie_lock.locked() and not acc.lock.locked()
            and not life.creation_lock.locked()
            and not any(t.get("in_use") and t.get("dev") == locked.id
                        for t in acc.token_pool.values()))
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
        and not any(d.users or (d is not locked and d.lock.locked())
                    for d in acc.dev_objs.values())
        and not any(t.get("in_use") for t in acc.token_pool.values()))


def _same_device(host, account, acc, device, context, page):
    return (host.accounts.get(account) is acc
        and host.lifecycles.get(account) is acc.lifecycle
        and acc.dev_objs.get(device.id) is device
        and device.ctx is context and device.page is page
        and acc.lifecycle.browser is acc.browser
        and acc.lifecycle.phase == "resident" and not acc.closing)


async def _close_idle(host, account, acc, device, allow_busy=False):
    if device.lock.locked():
        return False
    async with device.lock:
        if not _idle(host, account, acc, locked=device, allow_busy=allow_busy):
            return False
        context, page = device.ctx, device.page
        if context is None or page is None or device.id == acc.primary_dev:
            return False
        with acc.lifecycle.activity.activity() as maintenance:
            async def close_owned():
                # Host._rpc contributes the second activity and owns this task.
                if (not _idle(host, account, acc, device, maintenance,
                              asyncio.current_task(), allow_busy=allow_busy)
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
                except BaseException as error:
                    acc.closing = True
                    cause = events.error_cause(error)
                    acc.lifecycle.unconfirmed("closing an idle device context failed: " + cause)
                    events.emit("browser.cleanup", level="warn", outcome="unconfirmed",
                                error="idle-device-context-close-failed", cause=cause)
                    raise
                device.ctx, device.page, device.warm = None, None, False
                return True

            async with page_use(account):
                try:
                    return await asyncio.wait_for(
                        host._rpc(account, close_owned), CLOSE_TIMEOUT)
                except asyncio.TimeoutError:
                    cause = f"device context did not acknowledge closure within {CLOSE_TIMEOUT:g} seconds"
                    acc.closing = True
                    acc.lifecycle.unconfirmed(cause)
                    events.emit("browser.cleanup", level="warn", outcome="unconfirmed",
                                error="idle-device-context-close-timeout", cause=cause)
                    return False
                except Exception:
                    return False


async def trim_idle_device_contexts(host, account, acc, keep=2, *, allow_busy=False):
    """Close at most two excess contexts; keep the primary and recent devices."""
    if acc.retention_lock.locked():
        return 0
    async with acc.retention_lock:
        return await _trim_devices(host, account, acc, keep, allow_busy)


async def _trim_devices(host, account, acc, keep, allow_busy):
    if type(keep) is not int or keep < 1:
        raise ValueError("device context retention must be positive")
    closed = 0
    for _ in range(2):
        if not allow_busy and not _idle(host, account, acc):
            break
        resident = [d for d in acc.dev_objs.values()
                    if d.ctx is not None and d.page is not None]
        if len(resident) <= keep:
            break
        recent = sorted(resident, key=lambda d: (d.id == acc.primary_dev,
                                                d.last_used), reverse=True)
        candidates = [d for d in reversed(recent)
                      if d.id != acc.primary_dev and not d.users and not d.lock.locked()]
        if not candidates:
            break
        if not await _close_idle(host, account, acc, candidates[0], allow_busy):
            break
        closed += 1
    return closed
