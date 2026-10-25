"""NDJSON framing and asynchronous reply ownership."""
import asyncio, json, os

def reply_chunks(out):
    """Encode the existing NDJSON bytes without copying a large text body.

    Prepare every other field before writing, so a serialization error cannot
    leave half a JSON line in the shared pipe. The caller owns the writer lock.
    """
    result = out.get("result") if type(out) is dict else None
    body = result.get("body") if type(result) is dict else None
    if (type(body) is not str or len(body) < 65536
            or tuple(out) != ("id", "ok", "result")
            or len(result) > 16
            or any(type(key) is not str for key in result)):
        yield json.dumps(out) + "\n"
        return

    before = ['{"id": ', json.dumps(out["id"]),
              ', "ok": ', json.dumps(out["ok"]), ', "result": {']
    after = []
    parts = before
    for index, (key, value) in enumerate(result.items()):
        if index:
            parts.append(", ")
        parts.extend((json.dumps(key), ": "))
        if key == "body":
            parts.append('"')
            parts = after
            parts.append('"')
        else:
            parts.append(json.dumps(value))
    parts.append("}}\n")
    prefix, suffix = "".join(before), "".join(after)
    del before, after, parts
    yield prefix
    for start in range(0, len(body), 16384):
        # Python slices at code points. Default ASCII escaping also preserves
        # lone surrogates and matches the original encoder at chunk boundaries.
        yield json.dumps(body[start:start + 16384])[1:-1]
    yield suffix


def write_reply(stream, out):
    """Synchronous encoding adapter for offline measurements."""
    for chunk in reply_chunks(out):
        stream.write(chunk)


class AsyncReplyWriter:
    """Keep pipe backpressure off the browser loop, with one frame in flight.

    Cancellation before acquiring the lock discards the queued reply. Once a
    frame starts, its task owns the lock until the complete line is written.
    Only closing the entire writer may abandon a partially written frame.
    """

    def __init__(self, stream):
        self._fd = os.dup(stream.fileno())
        try:
            os.set_blocking(self._fd, False)
        except BaseException:
            os.close(self._fd)
            raise
        self._lock = asyncio.Lock()
        self._tasks = set()
        self._closed = False

    @property
    def busy(self):
        # A cancelled sender can leave its complete-frame task draining.
        return self._lock.locked()

    def _close_fd(self):
        if self._fd is not None:
            os.close(self._fd)
            self._fd = None

    def _finished(self, task):
        self._tasks.discard(task)
        self._lock.release()
        if not task.cancelled():
            # A cancelled caller no longer retrieves its writer's exception.
            task.exception()

    async def send(self, out):
        await self._lock.acquire()
        if self._closed:
            self._lock.release()
            raise BrokenPipeError("reply writer is closed")
        try:
            task = asyncio.create_task(self._write_frame(out))
        except BaseException:
            self._lock.release()
            raise
        self._tasks.add(task)
        task.add_done_callback(self._finished)
        await asyncio.shield(task)

    async def _write_frame(self, out):
        loop = asyncio.get_running_loop()
        fd = self._fd
        started = False
        try:
            for chunk in reply_chunks(out):
                data = memoryview(chunk.encode("ascii"))
                while data:
                    try:
                        count = os.write(fd, data)
                    except BlockingIOError:
                        ready = loop.create_future()

                        def writable():
                            if not ready.done():
                                ready.set_result(None)

                        loop.add_writer(fd, writable)
                        try:
                            await ready
                        finally:
                            loop.remove_writer(fd)
                        continue
                    if not count:
                        raise BrokenPipeError("reply pipe made no progress")
                    started = True
                    data = data[count:]
        except BaseException as exc:
            if started or isinstance(exc, (OSError, asyncio.CancelledError)):
                self._closed = True
                self._close_fd()
            raise

    async def close(self):
        self._closed = True
        tasks = tuple(self._tasks)
        for task in tasks:
            task.cancel()
        if tasks:
            await asyncio.gather(*tasks, return_exceptions=True)
        self._close_fd()
