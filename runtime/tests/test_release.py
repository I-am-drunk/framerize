"""A selected package cannot quietly mix runtime files from different releases."""
import hashlib
import json
from pathlib import Path
import sys
import subprocess
import tempfile
import tarfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from framerize_runtime.release import verify_runtime
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'scripts'))
import release as installer
import package as builder


def worker_fixture(root, commit=None):
    root.mkdir()
    (root / 'bin').mkdir()
    for name in ('bin/node', 'worker.mjs', 'worker-config.json'):
        (root / name).write_text('synthetic, never executed')
    commit = commit or subprocess.check_output(
        ['git', 'rev-parse', 'HEAD'], cwd=builder.ROOT, text=True).strip()
    (root / 'bundle-source.json').write_text(json.dumps({'commit': commit}))
    files = {p.relative_to(root).as_posix(): installer.sha256(p)
             for p in root.rglob('*') if p.is_file()}
    (root / 'bundle-manifest.json').write_text(json.dumps(files))
    return root


class ReleaseTests(unittest.TestCase):
    def test_worker_copy_refuses_tampering_missing_files_and_links(self):
        for change in ('contents', 'missing', 'extra', 'link'):
            with self.subTest(change=change), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                worker = worker_fixture(root / 'worker')
                entry = worker / 'worker.mjs'
                if change == 'contents':
                    entry.write_text('changed')
                elif change == 'missing':
                    entry.unlink()
                elif change == 'extra':
                    (worker / 'extra.mjs').touch()
                else:
                    entry.unlink()
                    entry.symlink_to('bin/node')
                with self.assertRaises(ValueError):
                    builder.copy_worker(worker, root / 'copy')

    def test_clean_debug_build_is_still_a_development_package(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / 'debug'
            target.mkdir()
            (target / 'framerize-native').write_bytes(b'synthetic executable')
            worker = worker_fixture(root / 'worker', 'f' * 40)
            def output(args, **kwargs):
                return {'rev-parse': 'f' * 40, 'status': b'', '--version': 'fixture rustc'}[args[1]]
            with patch.object(builder.subprocess, 'check_output', side_effect=output):
                with self.assertRaisesRegex(ValueError, 'release build'):
                    builder.package('framerize-native', root / 'out', target)
                archive = builder.package('framerize-native', root / 'out', target, True, worker)
            self.assertIn('-development.tar.gz', archive.name)
            with tarfile.open(archive) as bundle:
                self.assertTrue(any(m.name.endswith('/runtime/worker/bin/node') for m in bundle.getmembers()))
                member = next(m for m in bundle.getmembers() if m.name.endswith('/build-info.json'))
                info = json.load(bundle.extractfile(member))
            self.assertFalse(info['source_dirty'])
            self.assertTrue(info['development'])
            self.assertEqual(info['build_profile'], 'debug')

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
        (self.tree / 'runtime').mkdir()
        (self.tree / 'runtime/native.py').write_text('# fixture, never executed\n')
        (self.tree / 'runtime/runtime-manifest.json').write_text('{}')
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

    def test_verify_cli_checks_binary_through_current_symlink_without_execution(self):
        current = self.root / 'current'
        current.symlink_to(self.tree)
        command = [sys.executable, '-B', installer.__file__, 'verify', str(current)]
        valid = subprocess.run(command, capture_output=True, text=True, timeout=10)
        self.assertEqual(valid.returncode, 0, valid.stderr)
        self.assertEqual(json.loads(valid.stdout)['application'], 'framerize-native')
        binary = self.tree / 'bin/framerize-native'
        original_manifest = (self.tree / 'manifest.json').read_bytes()
        binary.write_text('older executable under the newer release name')
        changed = subprocess.run(command, capture_output=True, text=True, timeout=10)
        self.assertEqual(changed.returncode, 1)
        self.assertIn('release file checksum differs', changed.stderr)
        self.assertEqual((self.tree / 'manifest.json').read_bytes(), original_manifest)
        self.assertEqual(current.resolve(), self.tree)

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

    def test_native_package_requires_its_runtime_even_with_valid_checksums(self):
        for missing in ('native.py', 'runtime-manifest.json'):
            path = self.tree / 'runtime' / missing
            contents = path.read_bytes()
            path.unlink()
            manifest = self.tree / 'manifest.json'
            original = manifest.read_text()
            files = json.loads(original)
            del files['runtime/' + missing]
            manifest.write_text(json.dumps(files))
            with self.assertRaisesRegex(ValueError, 'incomplete application package'):
                installer.verify(self.tree)
            path.write_bytes(contents)
            manifest.write_text(original)

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
