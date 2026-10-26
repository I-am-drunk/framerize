"""Keep a borrowed page owned until its dispatched RPCs have settled."""
import asyncio
from contextlib import asynccontextmanager
from contextvars import ContextVar

_PAGE_CALLS = ContextVar("framerize_page_calls", default=None)


def record_page_call(account, task):
    scope = _PAGE_CALLS.get()
    if scope is not None and scope[0] == account:
        owned = scope[1]
        owned.add(task)
        task.add_done_callback(owned.discard)


@asynccontextmanager
async def page_use(account):
    owned = set()
    token = _PAGE_CALLS.set((account, owned))
    try:
        yield
    finally:
        _PAGE_CALLS.reset(token)
        await _join_calls(owned)


async def _join_calls(owned):
    cancelled = None
    while owned:
        joined = asyncio.gather(*tuple(owned), return_exceptions=True)
        # Each batch stays owned even if cancellation arrives more than once.
        while not joined.done():
            try:
                await asyncio.shield(joined)
            except asyncio.CancelledError as error:
                cancelled = error
        joined.result()
    if cancelled is not None:
        raise cancelled
