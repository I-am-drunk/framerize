"""A slow browser driver must not accumulate repeated outstanding polls."""
import asyncio
from pathlib import Path
import sys
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_host.browser import responses


class PollOwnershipTests(unittest.IsolatedAsyncioTestCase):
    async def test_unresponsive_driver_gets_one_poll_until_response_deadline(self):
        calls = []
        async def poll(acknowledged):
            calls.append(acknowledged)
            await asyncio.Event().wait()

        start = time.monotonic()
        clock = lambda: (time.monotonic() - start) * 50
        async def wait_for(awaitable, timeout):
            return await asyncio.wait_for(awaitable, timeout / 50)

        timers = SimpleNamespace(wait_for=wait_for, TimeoutError=asyncio.TimeoutError,
                                 sleep=asyncio.sleep)
        with patch.object(responses, 'time', SimpleNamespace(monotonic=clock)), \
                patch.object(responses, 'asyncio', timers):
            with self.assertRaises(responses.ResponseDeadline):
                await responses.receive(poll, 20)
        self.assertEqual(calls, [-1])
