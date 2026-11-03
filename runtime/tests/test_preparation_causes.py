"""Preparation failures retain their cause and cannot become inference retries."""
import unittest
import test_device_context_creation as creation


class PreparationCauseTests(unittest.IsolatedAsyncioTestCase):
    setUp = creation.DeviceContextCreationTests.setUp
    finish = creation.DeviceContextCreationTests.finish
    account = creation.DeviceContextCreationTests.account
    prepare = creation.DeviceContextCreationTests.prepare
    command = creation.DeviceContextCreationTests.command
    start = creation.DeviceContextCreationTests.start
    block = creation.DeviceContextCreationTests.block
    assert_idle = creation.DeviceContextCreationTests.assert_idle

    async def test_navigation_failure_is_local_and_closes_its_context(self):
        acc = self.account()
        context = acc.browser.next_context
        context.page.goto.side_effect = RuntimeError('Page.goto: Timeout 90000ms exceeded.')
        with self.assertRaisesRegex(RuntimeError, '^browser preparation failed:') as failure:
            await self.command()
        self.assertIn('Framer login page', str(failure.exception))
        self.assertIn('Timeout 90000ms exceeded', str(failure.exception))
        self.assertTrue(context.closed)
        context.page.evaluate.assert_not_awaited()
        context.page.goto.assert_awaited_once_with(self.module.LOGIN_URL,
            wait_until='domcontentloaded', timeout=90000)
        self.assertIsNone(acc.dev_objs['device-a'].ctx)
        self.assert_idle(acc)

    async def test_sdk_failure_keeps_local_classification_and_cleanup(self):
        acc = self.account()
        context = acc.browser.next_context
        context.page.evaluate.side_effect = RuntimeError('fixture SDK download failed')
        with self.assertRaisesRegex(RuntimeError, '^browser preparation failed:') as failure:
            await self.command()
        self.assertIn('Castle SDK warmup', str(failure.exception))
        self.assertIn('fixture SDK download failed', str(failure.exception))
        self.assertTrue(context.closed)
        self.assertIsNone(acc.dev_objs['device-a'].ctx)
        self.assert_idle(acc)
