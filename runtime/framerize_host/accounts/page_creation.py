"""Reclaim a page whose creation finishes after its caller is cancelled."""
import asyncio


async def create_page(make, close):
    creation = asyncio.create_task(make())
    try:
        return await asyncio.shield(creation)
    except asyncio.CancelledError:
        cleanup = asyncio.create_task(_close_created(creation, close))
        while not cleanup.done():
            try:
                await asyncio.shield(cleanup)
            except asyncio.CancelledError:
                continue
            except Exception:
                break
        # Cleanup failure stays an error; merely requesting close is not proof.
        cleanup.result()
        raise


async def _close_created(creation, close):
    try:
        page = await creation
    except (Exception, asyncio.CancelledError):
        return
    await close(page)
