"""Retain a device through its request and all dispatched browser calls."""
from contextlib import asynccontextmanager
import time

from .page_use import page_use
from .state import Device
from .device_retention import trim_idle_device_contexts
from ..events import emit


@asynccontextmanager
async def device_use(host, account, acc, device_id):
    device = acc.dev_objs.setdefault(device_id, Device(device_id))
    async with device.lock:
        if (host.accounts.get(account) is not acc or acc.closing
                or acc.lifecycle.phase != "resident"):
            raise RuntimeError(acc.lifecycle.cleanup_error())
        device.users += 1
    try:
        async with page_use(account):
            yield device
    finally:
        device.users -= 1
        device.last_used = time.monotonic()
        schedule_device_trim(host, account, acc)


def schedule_device_trim(host, account, acc):
    if (host._closed or acc.closing or acc.trim_task is not None
            or sum(d.ctx is not None for d in acc.dev_objs.values()) <= 2):
        return

    async def trim():
        try:
            while not host._closed and not acc.closing:
                closed = await trim_idle_device_contexts(
                    host, account, acc, allow_busy=True)
                if not closed:
                    return
                emit("memory.device_trim", closed=closed,
                     retained=sum(d.ctx is not None for d in acc.dev_objs.values()))
        finally:
            acc.trim_task = None

    acc.trim_task = host._track_task(acc, trim())
