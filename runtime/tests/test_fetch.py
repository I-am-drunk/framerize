"""Run the packaged job scripts with inert fetch and timer implementations."""
import json
from pathlib import Path
import shutil
import subprocess
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_host.browser import scripts


class FetchTests(unittest.TestCase):
    def test_body_completion_and_failure_phases(self):
        node = shutil.which('node')
        self.assertIsNotNone(node, 'Node is required for the browser script fixtures')
        values = {name: getattr(scripts, name) for name in
                  ('AI_START_JS', 'AI_POLL_JS', 'AI_CANCEL_JS')}
        result = subprocess.run([node, str(Path(__file__).with_name('ai_post_fetch.cjs'))],
            input=json.dumps(values), text=True, capture_output=True, timeout=10)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
