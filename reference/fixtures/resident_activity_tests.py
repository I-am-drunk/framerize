import asyncio
from pathlib import Path
import sys
import unittest
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'scripts'))
from resident_activity import ActivityLedger, ResidentRefused


def operation_id():
    return str(uuid.uuid4())


class ResidentActivityTests(unittest.IsolatedAsyncioTestCase):
    async def test_activity_and_lease_exclude_each_other(self):
        ledger = ActivityLedger()
        activity = ledger.activity()
        with self.assertRaises(ResidentRefused):
            ledger.acquire(operation_id())
        activity.close()
        lease = ledger.acquire(operation_id())
        with self.assertRaises(ResidentRefused):
            ledger.activity()
        verdict = await lease.settle('not_started', True)
        self.assertEqual((verdict.state, verdict.browser_settled), ('released', True))
        with ledger.activity():
            pass

    async def test_cancelled_activity_waiter_retains_rpc_until_actual_settlement(self):
        ledger = ActivityLedger()
        activity = ledger.activity()
        started, release = asyncio.Event(), asyncio.Event()

        async def rpc():
            started.set()
            await release.wait()
            return 'synthetic-result'

        waiter = asyncio.create_task(activity.rpc(rpc))
        await started.wait()
        waiter.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await waiter
        activity.close()
        self.assertEqual(len(ledger.pending), 1)
        with self.assertRaises(ResidentRefused):
            ledger.acquire(operation_id())
        release.set()
        await asyncio.gather(*ledger.pending)
        self.assertFalse(ledger.pending)
        lease = ledger.acquire(operation_id())
        self.assertEqual((await lease.settle('not_started', True)).state, 'released')

    async def test_dropped_settlement_waiter_keeps_one_completion_owner(self):
        ledger = ActivityLedger()
        lease = ledger.acquire(operation_id())
        started, release = asyncio.Event(), asyncio.Event()

        async def rpc():
            started.set()
            await release.wait()

        mint = asyncio.create_task(lease.rpc(rpc))
        await started.wait()
        waiter = asyncio.create_task(lease.settle('settled', True))
        await asyncio.sleep(0)
        completion = lease.completion
        self.assertIsNotNone(completion)
        self.assertFalse(completion.done())
        waiter.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await waiter
        self.assertIs(ledger.current, lease)
        self.assertFalse(completion.cancelled())
        release.set()
        await mint
        verdict = await lease.settle('settled', True)
        self.assertEqual(verdict.state, 'released')
        self.assertIs(lease.completion, completion)
        self.assertIs(await lease.settle('settled', True), verdict)

    async def test_unknown_native_completion_fences_only_its_account(self):
        ledger, other = ActivityLedger(), ActivityLedger()
        lease = ledger.acquire(operation_id())
        verdict = await lease.settle('unknown', True)
        self.assertEqual((verdict.state, verdict.browser_settled), ('fenced', True))
        self.assertIs(await lease.settle('settled', True), verdict)
        with self.assertRaises(ResidentRefused):
            ledger.activity()
        with self.assertRaises(ResidentRefused):
            ledger.acquire(operation_id())
        with other.activity():
            pass

    async def test_replay_and_capacity_never_evict_old_operation(self):
        ledger = ActivityLedger(capacity=1)
        identity = operation_id()
        lease = ledger.acquire(identity)
        await lease.settle('not_started', True)
        for attempt in (identity, operation_id()):
            with self.assertRaises(ResidentRefused):
                ledger.acquire(attempt)
        self.assertEqual(list(ledger.leases), [identity])

    async def test_cancelled_actual_rpc_retains_account_fence(self):
        ledger = ActivityLedger()
        lease = ledger.acquire(operation_id())
        waiter = asyncio.create_task(lease.rpc(lambda: asyncio.sleep(60)))
        await asyncio.sleep(0)
        next(iter(ledger.pending)).cancel()
        with self.assertRaises(asyncio.CancelledError):
            await waiter
        self.assertEqual((await lease.settle('not_started', True)).state, 'fenced')

    async def test_closed_mint_admission_does_not_create_an_rpc(self):
        ledger = ActivityLedger()
        lease = ledger.acquire(operation_id())
        await lease.settle('not_started', True)

        def forbidden():
            self.fail('RPC factory must not run after settlement starts')

        with self.assertRaises(ResidentRefused):
            await lease.rpc(forbidden)
        self.assertFalse(ledger.pending)

    async def test_malformed_identifiers_and_settlement_preserve_owner(self):
        ledger = ActivityLedger()
        for value in (None, 1, True, {}, '', 'not-a-uuid', uuid.uuid4().hex):
            with self.assertRaises(ResidentRefused):
                ledger.acquire(value)
        self.assertFalse(ledger.leases)
        lease = ledger.acquire(operation_id())
        for state, resume in [('unknown', 1), ('completed', True), ({}, True)]:
            with self.assertRaises(ResidentRefused):
                await lease.settle(state, resume)
            self.assertIs(ledger.current, lease)
            self.assertIsNone(lease.completion)
        await lease.settle('not_started', True)


if __name__ == '__main__':
    unittest.main()
