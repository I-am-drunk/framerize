"""Inert checks for bounded, privacy-preserving preparation checkpoints."""
import asyncio
import ast
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import time
from types import SimpleNamespace
import unittest

SPEC = importlib.util.spec_from_file_location(
    '_progress_diagnostics', Path(__file__).with_name('browser_comparison_diagnostics_tests.py'))
D = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(D)
C = D.C


class ProgressTests(unittest.TestCase):
    def test_real_file_sink_is_readable_before_command_completion(self):
        source = Path(__file__).parents[2] / 'scripts/camoufox_host.py'
        tree = ast.parse(source.read_text())
        node = next(n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == 'emit')
        with tempfile.TemporaryDirectory() as directory:
            namespace = {'LOG_DIR': directory, 'os': os, 'time': time, 'json': json}
            exec(compile(ast.Module(body=[node], type_ignores=[]), str(source), 'exec'), namespace)
            scope = SimpleNamespace()
            C.comparison_attach_progress(scope, {'purpose': 'baseline_context'}, namespace['emit'])
            C.comparison_mark(scope, 'context_ready', token=D.SECRET)
            files = list(Path(directory).glob('events-*.ndjson'))
            self.assertEqual(len(files), 1)
            rows = [json.loads(line) for line in files[0].read_text().splitlines()]
            self.assertEqual(len(rows), 1)
            self.assertEqual(rows[0]['event'], 'browser.comparison.progress')
            self.assertEqual(rows[0]['stage'], 'context_ready')
            self.assertEqual(rows[0]['sequence'], 1)
            self.assertNotIn(D.SECRET, files[0].read_text())

    def test_sequence_persists_across_commands_and_payload_is_allowlisted(self):
        scope, events = SimpleNamespace(), []
        def emit(event, **fields): events.append(dict(event=event, **fields))
        for purpose in ('baseline_context', 'native_token'):
            C.comparison_attach_progress(scope, {'purpose': purpose, 'token': D.SECRET}, emit)
            C.comparison_mark(scope, 'mint_rpc', device_valid=True, token=D.SECRET)
        self.assertEqual([e['sequence'] for e in events], [1, 2])
        self.assertEqual([e['purpose'] for e in events], ['baseline_context', 'native_token'])
        self.assertEqual(events[0]['trace'], events[1]['trace'])
        self.assertRegex(events[0]['trace'], r'^[0-9a-f]{32}$')
        self.assertEqual(set(events[0]), {'event', 'trace', 'sequence', 'purpose', 'stage', 'at_limit'})
        self.assertNotIn(D.SECRET, json.dumps(events))

    def test_unknown_values_are_fixed_and_repeated_marks_are_capped(self):
        scope, events = SimpleNamespace(), []
        C.comparison_attach_progress(scope, {'purpose': D.SECRET},
            lambda event, **fields: events.append(fields))
        for _ in range(200): C.comparison_mark(scope, D.SECRET)
        self.assertEqual(len(events), 128)
        self.assertEqual([e['sequence'] for e in events], list(range(1, 129)))
        self.assertTrue(events[-1]['at_limit'])
        self.assertTrue(all(e['purpose'] == e['stage'] == 'unknown' for e in events))
        self.assertNotIn(D.SECRET, json.dumps(events))

    def test_sink_failure_cannot_change_diagnostic_or_cleanup_state(self):
        scope = SimpleNamespace(failure=None)
        def broken(*args, **fields): raise RuntimeError(D.SECRET)
        C.comparison_attach_progress(scope, {'cmd': 'browser_baseline_once'}, broken)
        C.comparison_mark(scope, 'mint_post_page')
        self.assertEqual(C.comparison_evidence(scope), {'stage': 'mint_post_page'})
        self.assertEqual(scope.comparison_progress_sequence, 1)
        self.assertIsNone(scope.failure)

    def test_scopes_have_distinct_trace_ids(self):
        scopes = [SimpleNamespace(), SimpleNamespace()]
        for scope in scopes: C.comparison_attach_progress(scope, {}, lambda *a, **k: None)
        self.assertNotEqual(scopes[0].comparison_progress_id, scopes[1].comparison_progress_id)


class ProgressWorkflowTests(unittest.IsolatedAsyncioTestCase):
    asyncSetUp = D.FIXTURE.BrowserComparisonTests.asyncSetUp
    asyncTearDown = D.FIXTURE.BrowserComparisonTests.asyncTearDown
    fixture = D.ComparisonDiagnosticWorkflowTests.fixture

    async def test_pending_mint_checkpoint_precedes_command_return(self):
        value = self.fixture()
        await value.prepare_context()
        value.page.sdk_release.clear()
        task = asyncio.create_task(value.baseline())
        self.tasks.append(task)
        await asyncio.wait_for(value.page.sdk_started.wait(), 2)
        self.assertFalse(task.done())
        self.assertEqual(value.progress[-1]['stage'], 'mint_rpc')
        self.assertEqual(value.progress[-1]['purpose'], 'baseline')
        self.assertEqual(len(value.events), 1)
        task.cancel()
        with self.assertRaises(asyncio.CancelledError): await task
        value.page.sdk_release.set()
        self.assertEqual(value.fetches, 0)
        self.assertNotIn(D.SECRET, json.dumps(value.progress))

    async def test_pair_progress_has_one_trace_and_ordered_phases(self):
        value = self.fixture()
        await value.prepare_context()
        self.assertTrue((await value.baseline())['accepted'])
        await value.native()
        self.assertEqual(len({e['trace'] for e in value.progress}), 1)
        self.assertEqual([e['sequence'] for e in value.progress], list(range(1, len(value.progress) + 1)))
        self.assertEqual({e['purpose'] for e in value.progress}, {'baseline_context', 'baseline', 'native_token'})
        self.assertEqual(len(value.events), 3)
