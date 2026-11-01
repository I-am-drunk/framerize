import errno
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from framerize_host import events, resources
from framerize_host.accounts.state import AccountLifecycle


class ErrorCauseTests(unittest.TestCase):
    def test_invalid_browser_patch_names_the_integrity_failure_and_cannot_fall_back(self):
        with tempfile.TemporaryDirectory() as directory:
            (Path(directory) / 'framerize-memory-patch.json').write_text(
                json.dumps({'patch_version': 999}))
            with patch.dict(os.environ, FRAMERIZE_MEMORY_BROWSER=directory), \
                    patch.object(resources, 'emit'):
                with self.assertRaisesRegex(RuntimeError, 'unsupported memory browser patch'):
                    resources.prepared_memory_browser()

    def test_first_cleanup_failure_survives_later_failures_without_credentials(self):
        life = AccountLifecycle()
        life.unconfirmed("close failed: broken pipe; Authorization: Bearer private-fixture")
        life.unconfirmed("later generic closure error")
        self.assertIn("broken pipe", life.cleanup_error())
        self.assertNotIn("private-fixture", life.cleanup_error())
        self.assertNotIn("later generic", life.cleanup_error())

    def test_disk_full_reports_the_io_cause_once_without_changing_the_request(self):
        output = io.StringIO()
        with tempfile.TemporaryDirectory() as directory:
            with patch.multiple(events, LOG_DIR=directory, _last_log_failure=None):
                with patch('builtins.open', side_effect=OSError(errno.ENOSPC, 'No space left on device')):
                    with patch.object(events.sys, 'stderr', output):
                        events.emit('fixture', private_input='must-not-appear')
                        events.emit('fixture')
        self.assertEqual(output.getvalue().count('event log write failed'), 1)
        self.assertIn('No space left on device', output.getvalue())
        self.assertNotIn('must-not-appear', output.getvalue())
