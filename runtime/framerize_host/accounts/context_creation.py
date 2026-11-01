"""Retain device context creation and cancellation cleanup."""
import asyncio

from .. import events
from .page_creation import create_page


async def create_device_context(acc, make, close):
    async def close_cancelled(context):
        await close_cancelled_device_context(acc, lambda: close(context))

    return await create_page(make, close_cancelled)


async def close_cancelled_device_context(
        acc, close, *, reason="cancelled-device-context-close-failed"):
    """Join cleanup and return cancellation received while it was running."""
    cancelled = None
    closing = asyncio.create_task(close())
    while not closing.done():
        try:
            await asyncio.shield(closing)
        except asyncio.CancelledError as error:
            cancelled = error
            continue
        except Exception:
            break
    try:
        closing.result()
    except (Exception, asyncio.CancelledError) as error:
        # An ordinary error would enter the device mint's fallback. Keep
        # cancellation and require browser cleanup before new admission.
        acc.closing = True
        cause = events.error_cause(error)
        acc.lifecycle.unconfirmed(reason + ": " + cause)
        events.emit("browser.cleanup", level="warn", outcome="unconfirmed",
                    error=reason, cause=cause)
    return cancelled
