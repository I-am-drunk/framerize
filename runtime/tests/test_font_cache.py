"""Font cache advice must preserve files and remain an optional optimization."""
import errno
import os
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_host.browser import font_cache


class FontCacheTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.fonts = self.root / 'fonts'
        self.fonts.mkdir()
        self.font = self.fonts / 'font.ttf'
        self.font.write_bytes(b'unchanged font bytes')

    def test_only_regular_bundled_fonts_are_advised_without_changes(self):
        other = self.root / 'unrelated'
        other.write_bytes(b'other file')
        (self.fonts / 'linked.ttf').symlink_to(other)
        (self.fonts / 'linked-dir').symlink_to(self.root, target_is_directory=True)
        seen = []
        def advise(fd, offset, length, mode):
            self.assertEqual((offset, length, mode), (0, 0, os.POSIX_FADV_DONTNEED))
            seen.append(os.read(fd, 100))
        with patch.object(font_cache.os, 'posix_fadvise', side_effect=advise):
            result = font_cache.release_font_cache(self.root / 'camoufox')
        self.assertEqual(seen, [b'unchanged font bytes'])
        self.assertEqual(result, dict(outcome='ok', advised_files=1))
        self.assertEqual(self.font.read_bytes(), b'unchanged font bytes')
        self.assertEqual(other.read_bytes(), b'other file')

    def test_failed_advice_keeps_the_underlying_cause(self):
        with patch.object(font_cache.os, 'posix_fadvise',
                          side_effect=OSError(errno.EIO, 'Input/output error')):
            result = font_cache.release_font_cache(self.root / 'camoufox')
        self.assertEqual(result['outcome'], 'unavailable')
        self.assertIn('Input/output error', result['cause'])
        self.assertEqual(self.font.read_bytes(), b'unchanged font bytes')

    def test_missing_platform_support_does_not_fail_startup(self):
        with patch.object(font_cache, 'os', SimpleNamespace()):
            result = font_cache.release_font_cache(self.root / 'camoufox')
        self.assertEqual(result, dict(outcome='unsupported', advised_files=0))
