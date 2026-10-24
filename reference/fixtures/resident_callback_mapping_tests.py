"""Inert event contract tests using installed Playwright, without a browser."""
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import resident_http200_provenance_tests as fixture
from playwright._impl._impl_to_api_mapping import ImplToApiMapping


class ResidentCallbackMappingTests(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.fixture = fixture.ResidentHttp200Tests()
        await self.fixture.asyncSetUp()
        mapper = ImplToApiMapping()
        original_on = fixture.Events.on
        original_remove = fixture.Events.remove_listener
        self.on = patch.object(fixture.Events, 'on', lambda obj, event, callback:
            original_on(obj, event, mapper.wrap_handler(callback)))
        self.remove = patch.object(fixture.Events, 'remove_listener', lambda obj, event, callback:
            original_remove(obj, event, mapper.wrap_handler(callback)))
        self.on.start()
        self.remove.start()
        self.addCleanup(self.on.stop)
        self.addCleanup(self.remove.stop)

    async def test_real_callback_mapping_records_http200_then_invalidates_navigation(self):
        f = self.fixture
        await f.host._handle(f.request())
        evidence = f.acc.completed_browser_http200
        self.assertIsNotNone(evidence)
        f.device.page.fire('framenavigated', f.device.page.main_frame)
        self.assertTrue(evidence.invalid)
        self.assertIsNone(f.acc.completed_browser_http200)
        f.assert_no_watchers()

    async def test_real_callback_mapping_preserves_subframes_and_detaches_close(self):
        f = self.fixture
        await f.host._handle(f.request())
        evidence = f.acc.completed_browser_http200
        self.assertIsNotNone(evidence)
        f.device.page.fire('framenavigated', object())
        self.assertIs(f.acc.completed_browser_http200, evidence)
        f.device.ctx.fire('close')
        self.assertIsNone(f.acc.completed_browser_http200)
        f.assert_no_watchers()

    async def test_real_callback_mapping_detaches_refused_response(self):
        f = self.fixture
        f.result['status'] = 500
        await f.host._handle(f.request())
        self.assertIsNone(f.acc.completed_browser_http200)
        f.assert_no_watchers()
        self.assertEqual((len(f.sends), len(f.mints)), (1, 1))


if __name__ == '__main__':
    unittest.main()
