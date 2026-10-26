"""Exercise the real startup path against disposable history only."""
from contextlib import redirect_stderr
import io
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest
from unittest.mock import AsyncMock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_accounts import diagnostics, runtime_owner
from framerize_accounts.rates import SystemClock
from framerize_host import __main__ as entry, app, events, settings


class HistoryStartupTests(unittest.TestCase):
    def test_diagnostics_identify_the_action_without_repeating_input(self):
        for message, reason in [
                ('incompatible account rate policy', 'rate-policy-mismatch'),
                ('existing rate history required', 'history-migration-required'),
                ('expiry requires the successor service runtime', 'history-owner-unconfirmed')]:
            self.assertEqual(diagnostics.describe(RuntimeError(message))['reason'], reason)
        output = io.StringIO()
        with redirect_stderr(output):
            diagnostics.report(ValueError('session=fixture-secret'))
        self.assertNotIn('fixture-secret', output.getvalue())
        self.assertIn('action', output.getvalue())

    def test_unrecognized_history_reports_a_safe_action_without_starting_work(self):
        path = self.root / 'rate-history.sqlite3'
        with sqlite3.connect(path) as db:
            db.execute('CREATE TABLE private_state (secret TEXT)')
            db.execute("INSERT INTO private_state VALUES ('session=fixture-secret')")
        db.close()
        before, output = path.read_bytes(), io.StringIO()
        with patch.object(app.Host, 'loop', new_callable=AsyncMock) as loop, redirect_stderr(output):
            with self.assertRaises(SystemExit) as result:
                entry.main('fixture.py')
        self.assertEqual(result.exception.code, 1)
        loop.assert_not_awaited()
        self.assertIn('unsupported-history-schema', output.getvalue())
        self.assertNotIn('fixture-secret', output.getvalue())
        self.assertEqual(path.read_bytes(), before)

    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        for module, key, value in [(app, 'ROOT', str(self.root)),
                (settings, 'ROOT', str(self.root)), (entry, 'verify_runtime', lambda _: None)]:
            self.enterContext(patch.object(module, key, value))
        self.emitted = self.enterContext(patch.object(events, 'emit'))
        self.enterContext(patch.object(events, 'trace'))

    def test_missing_history_is_recovered_before_the_host_loop(self):
        (self.root / 'account').mkdir()
        (self.root / 'account/framerize-devices.json').write_text('["stable"]')
        owner = dict(boot_id=SystemClock().boot_id, invocation='a' * 32)
        async def inspect(host):
            self.assertEqual(await host.handle(dict(cmd='history_status', accounts=['account'])),
                             dict(ready=True, accounts=1))
            self.assertIsNone(host._browser_runtime)
            self.assertIsNone(host.lifecycles['account'].rates.allocate_reserve())
        with patch.object(runtime_owner, 'capture', return_value=owner), patch(
                'framerize_accounts.journal_recovery.parent_owns_sessions', return_value=True), patch.object(
                app.Host, 'loop', inspect):
            entry.main('fixture.py')
        self.emitted.assert_called_once_with('history.recovered', accounts=1,
                                             cooldown_seconds=600, reserve_hold_seconds=86400)
