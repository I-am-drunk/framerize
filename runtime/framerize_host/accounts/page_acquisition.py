"""Keep an already-started page allocation owned until it can be reclaimed."""
import asyncio


async def acquire_page(make, reclaim):
    task = asyncio.create_task(make())
    try:
        return await asyncio.shield(task)
    except asyncio.CancelledError:
        # Keep the caller's account lock and allocation scope alive. Repeated
        # cancellation must not detach the page before its result is owned.
        while not task.done():
            try:
                await asyncio.shield(task)
            except asyncio.CancelledError:
                continue
            except Exception:
                break
        if not task.cancelled() and task.exception() is None:
            reclaim(task.result())
        raise
