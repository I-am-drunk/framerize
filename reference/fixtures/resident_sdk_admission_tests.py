"""Resident SDK admission fixtures. Fake browser ports, no SDK or network."""
import asyncio
import unittest
import resident_crossover_tests as base
from resident_activity import ResidentRefused
from resident_authorization import IDENTITY, validate_request


class ResidentSdkAdmissionTests(unittest.IsolatedAsyncioTestCase):
    asyncSetUp = base.ResidentCrossoverTests.asyncSetUp
    asyncTearDown = base.ResidentCrossoverTests.asyncTearDown
    phase = base.ResidentCrossoverTests.phase
    baseline = base.ResidentCrossoverTests.baseline
    acquired = base.ResidentCrossoverTests.acquired
    passed = base.ResidentCrossoverTests.passed

    async def begin(self):
        return await self.control.command(self.phase('begin_native_sdk'))

    async def complete(self):
        return await self.control.command(self.phase('complete_native_sdk'))

    async def test_charge_before_worker_and_verify_without_browser_mint(self):
        baseline = await self.passed()
        begun = await self.begin()
        self.assertEqual(begun['state'], 'sdk_admitted')
        self.assertEqual(len(self.acc.dev_reqs[None]), 2)
        self.assertTrue(self.acc.mint_lock.locked())
        done = await self.complete()
        self.assertEqual(done['state'], 'sdk_complete')
        self.assertEqual(done['cookie_header'], baseline['cookie_header'])
        self.assertEqual(len(self.acc.dev_reqs[None]), 2)
        self.assertEqual(len(self.page.tokens), 1)
        self.assertEqual(sum(k == 'fetch' for k, _ in self.page.calls), 1)
        self.assertNotIn('token', begun)
        self.assertNotIn('token', done)
        self.assertFalse(self.acc.mint_lock.locked())

    async def test_order_and_both_native_branches_are_single_use(self):
        await self.acquired()
        for action in ('begin_native_sdk', 'complete_native_sdk'):
            with self.assertRaises(ResidentRefused):
                await self.control.command(self.phase(action))
        await self.control.command(self.baseline())
        await self.begin()
        for action in ('begin_native_sdk', 'mint_native', 'baseline'):
            with self.assertRaises(ResidentRefused):
                await self.control.command(self.phase(action))
        await self.complete()
        for action in ('begin_native_sdk', 'complete_native_sdk', 'mint_native'):
            with self.assertRaises(ResidentRefused):
                await self.control.command(self.phase(action))
        self.assertEqual(len(self.page.tokens), 1)

    async def test_browser_native_branch_prevents_sdk_admission(self):
        await self.passed()
        await self.control.command(self.phase('mint_native'))
        with self.assertRaises(ResidentRefused):
            await self.begin()
        self.assertEqual(len(self.acc.dev_reqs[None]), 2)

    async def test_cooldown_refuses_without_charging(self):
        await self.passed()
        self.acc.dev_cooling[None] = base.time.monotonic() + 600
        with self.assertRaises(ResidentRefused):
            await self.begin()
        self.assertEqual(len(self.acc.dev_reqs[None]), 1)

    async def test_finish_after_failed_worker_keeps_charge_and_fence(self):
        await self.passed()
        await self.begin()
        receipt = await self.control.command(self.phase('finish',
            native_state='settled', admission='fenced'))
        self.assertTrue(receipt['browser_settled'])
        self.assertEqual(receipt['state'], 'fenced')
        self.assertEqual(set(receipt), IDENTITY | {'state', 'browser_settled'})
        self.assertEqual(len(self.acc.dev_reqs[None]), 2)
        self.assertFalse(self.acc.mint_lock.locked())
        self.assertEqual(len(self.page.tokens), 1)

    async def test_capacity_refuses_without_charging(self):
        await self.passed()
        self.host._note_request(self.acc, None)
        self.host._note_request(self.acc, None)
        with self.assertRaises(ResidentRefused):
            await self.begin()
        self.assertEqual(len(self.acc.dev_reqs[None]), 3)

    def test_new_actions_are_exact_v2_only(self):
        for action in ('begin_native_sdk', 'complete_native_sdk'):
            req = self.phase(action)
            self.assertEqual(validate_request(req), req)
            for invalid in (dict(req, version=1), dict(req, token='private')):
                with self.assertRaises(ResidentRefused):
                    validate_request(invalid)

    async def test_changed_pin_after_worker_fences_without_another_mint(self):
        await self.passed()
        await self.begin()
        self.page.pin.valid = False
        with self.assertRaises(ResidentRefused):
            await self.complete()
        op = self.life.activity.current._resident_operation
        await op.completion
        self.assertTrue(self.life.activity.fenced)
        self.assertFalse(self.acc.mint_lock.locked())
        self.assertEqual(len(self.page.tokens), 1)

    async def test_cancelled_begin_retains_and_joins_rate_lock_owner(self):
        await self.passed()
        await self.acc.mint_lock.acquire()
        task = asyncio.create_task(self.begin())
        await asyncio.sleep(0)
        await asyncio.sleep(0)
        task.cancel()
        with self.assertRaises(asyncio.CancelledError):
            await task
        self.assertTrue(self.acc.mint_lock.locked())
        self.acc.mint_lock.release()
        op = self.life.activity.current._resident_operation
        await op.completion
        self.assertFalse(self.acc.mint_lock.locked())
        self.assertTrue(self.life.activity.fenced)
        self.assertEqual(len(self.acc.dev_reqs[None]), 1)


if __name__ == '__main__':
    unittest.main()
