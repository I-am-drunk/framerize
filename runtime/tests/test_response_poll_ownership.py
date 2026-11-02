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
    async def test_acknowledgement_wait_still_expires(self):
        clock = [0.0]
        async def poll(ack):
            if ack == -1:
                clock[0] = 1.01
                return dict(kind='chunk', sequence=0, offset=0, next=1,
                    body='A', status=200, usage=None, done=True, peak_units=1)
            clock[0] = 4.0
            raise asyncio.TimeoutError()
        with patch.object(responses, 'time', SimpleNamespace(monotonic=lambda: clock[0])):
            with self.assertRaises(responses.ResponseDeadline) as caught:
                await responses.receive(poll, 1.0)
        self.assertEqual(caught.exception.stage, 'acknowledgement')

    async def test_complete_body_gets_a_separate_acknowledgement_deadline(self):
        clock = [0.0]
        async def poll(ack):
            clock[0] = 1.01
            if ack == -1:
                return dict(kind='chunk', sequence=0, offset=0, next=1,
                    body='A', status=200, usage=None, done=True, peak_units=1)
            return dict(kind='closed', sequence=0)
        with patch.object(responses, 'time', SimpleNamespace(monotonic=lambda: clock[0])):
            result = await responses.receive(poll, 1.0)
        self.assertEqual(result, dict(status=200, usage=None, body='A'))

    async def test_finished_timeout_polls_back_off_until_the_deadline(self):
        calls = 0
        async def poll(acknowledged):
            nonlocal calls
            calls += 1
            raise asyncio.TimeoutError()
        with self.assertRaises(responses.ResponseDeadline):
            await responses.receive(poll, time.monotonic() + 0.06)
        self.assertGreater(calls, 0)
        self.assertLessEqual(calls, 8)

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
