"""A selected package cannot quietly mix runtime files from different releases."""
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import tarfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_host.release import verify_runtime
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'scripts'))
import release as installer


class ReleaseTests(unittest.TestCase):
    def test_manifest_detects_changed_missing_and_extra_runtime_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            host = root / 'host.py'
            host.write_text('# original\n')
            manifest = root / 'runtime-manifest.json'
            manifest.write_text(json.dumps({'host.py': hashlib.sha256(host.read_bytes()).hexdigest()}))
            self.assertEqual(verify_runtime(root), hashlib.sha256(manifest.read_bytes()).hexdigest())
            host.write_text('# changed\n')
            with self.assertRaisesRegex(ValueError, 'file changed'):
                verify_runtime(root)
            host.unlink()
            with self.assertRaisesRegex(ValueError, 'inventory changed'):
                verify_runtime(root)
            host.write_text('# original\n')
            (root / 'extra.py').touch()
            with self.assertRaisesRegex(ValueError, 'inventory changed'):
                verify_runtime(root)


class StageTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        self.tree = self.root / 'source/fixture-release'
        (self.tree / 'bin').mkdir(parents=True)
        (self.tree / 'bin/framerize-native').write_text('fixture, never executed')
        (self.tree / 'build-info.json').write_text(json.dumps(
            {'format': 1, 'application': 'framerize-native'}))
        files = {p.relative_to(self.tree).as_posix(): installer.sha256(p)
                 for p in self.tree.rglob('*') if p.is_file()}
        (self.tree / 'manifest.json').write_text(json.dumps(files))
        self.archive = self.root / 'fixture.tar.gz'

    def pack(self):
        with tarfile.open(self.archive, 'w:gz') as archive:
            archive.add(self.tree, arcname=self.tree.name)
        return installer.sha256(self.archive)

    def test_staging_keeps_current_release_and_refuses_overwrite(self):
        releases = self.root / 'releases'
        current = self.root / 'current'
        current.symlink_to('old-release')
        digest = self.pack()
        result = installer.stage(self.archive, digest, releases)
        self.assertEqual(result, releases / 'fixture-release')
        self.assertEqual(current.readlink(), Path('old-release'))
        self.assertEqual(installer.verify(result)['application'], 'framerize-native')
        with self.assertRaisesRegex(ValueError, 'already exists'):
            installer.stage(self.archive, digest, releases)

    def test_checksum_failure_creates_no_release_directory(self):
        self.pack()
        releases = self.root / 'releases'
        with self.assertRaisesRegex(ValueError, 'archive checksum'):
            installer.stage(self.archive, '0' * 64, releases)
        self.assertFalse(releases.exists())

    def test_changed_contents_and_links_are_refused_before_installation(self):
        for link in (False, True):
            with self.subTest(link=link):
                binary = self.tree / 'bin/framerize-native'
                binary.unlink()
                if link:
                    binary.symlink_to('/outside')
                else:
                    binary.write_text('changed')
                digest = self.pack()
                with self.assertRaises(ValueError):
                    installer.stage(self.archive, digest, self.root / 'releases')
                self.assertFalse((self.root / 'releases/fixture-release').exists())
