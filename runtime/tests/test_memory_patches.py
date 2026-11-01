"""Verify the pinned runtime repair without accounts or external requests."""
import errno
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
import zipfile
from unittest.mock import patch

from framerize_host.browser import driver_memory
from framerize_host.browser import memory_build
from framerize_host.browser.files import link_or_copy


class MemoryPatchTests(unittest.TestCase):
    def test_pinned_browser_teardown_preserves_source_and_other_pages(self):
        selected = os.environ.get('FRAMERIZE_TEST_BROWSER')
        if not selected:
            self.skipTest('set FRAMERIZE_TEST_BROWSER for installed-runtime qualification')
        source = Path(selected)
        before = memory_build.digest(source / 'omni.ja')
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / 'browser'
            memory_build.prepare(source, target)
            self.assertEqual(memory_build.digest(source / 'omni.ja'), before)
            with zipfile.ZipFile(target / 'omni.ja') as archive:
                module = root / 'TargetRegistry.js'
                module.write_bytes(archive.read(memory_build.CONTEXTS))
            result = subprocess.run([shutil.which('node'),
                str(Path(__file__).with_name('context_memory.cjs')), str(module)],
                capture_output=True, text=True, timeout=30)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_read_only_or_cross_filesystem_inputs_are_copied_without_changes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'source'
            source.write_bytes(b'fixture runtime')
            for code in (errno.EPERM, errno.EACCES, errno.EXDEV):
                target = root / str(code)
                with patch('os.link', side_effect=OSError(code, 'fixture')):
                    link_or_copy(source, target)
                self.assertEqual(target.read_bytes(), source.read_bytes())
                self.assertNotEqual(target.stat().st_ino, source.stat().st_ino)

    def test_unknown_driver_is_refused_without_publishing(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'source'
            (source / driver_memory.MODULE).parent.mkdir(parents=True)
            (source / driver_memory.MODULE).write_text('unsupported fixture')
            with self.assertRaisesRegex(ValueError, 'unsupported Playwright'):
                driver_memory.prepare(source, root / 'candidate')
            self.assertFalse((root / 'candidate').exists())

    def test_pinned_driver_preserves_inputs_and_releases_retained_objects(self):
        selected = os.environ.get('FRAMERIZE_TEST_DRIVER')
        if not selected:
            self.skipTest('set FRAMERIZE_TEST_DRIVER for installed-runtime qualification')
        source = Path(selected)
        before = driver_memory.digest(source / driver_memory.MODULE)
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory) / 'driver'
            driver_memory.prepare(source, target)
            self.assertEqual(driver_memory.digest(source / driver_memory.MODULE), before)
            self.assertEqual(driver_memory.digest(target / driver_memory.MODULE), driver_memory.PATCHED)
            self.assertNotEqual((source / driver_memory.MODULE).stat().st_ino,
                                (target / driver_memory.MODULE).stat().st_ino)
            result = subprocess.run([shutil.which('node'),
                str(Path(__file__).with_name('driver_memory.cjs')),
                str(target / driver_memory.MODULE)], capture_output=True, text=True, timeout=30)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            with self.assertRaisesRegex(ValueError, 'already exists'):
                driver_memory.prepare(source, target)
