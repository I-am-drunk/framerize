"""Compile and execute the actual Cargo build script in a disposable Git repo."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class BuildIdentityTests(unittest.TestCase):
    def test_clean_dirty_and_linked_worktree_identity(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            script = root / 'identity'
            subprocess.run(['rustc', str(ROOT / 'scripts/build_identity.rs'), '-o', str(script)],
                           check=True, capture_output=True, timeout=30)
            repo = root / 'repo'
            app = repo / 'apps/framerize'
            app.mkdir(parents=True)
            source = app / 'Cargo.toml'
            source.write_text('fixture')
            def git(*args):
                return subprocess.check_output(['git', '-C', str(repo), *args], text=True).strip()
            git('init', '-q')
            git('add', '.')
            git('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                '-c', 'commit.gpgsign=false', 'commit', '-qm', 'fixture')
            commit = git('rev-parse', 'HEAD')
            def run(manifest):
                return subprocess.check_output([str(script)], text=True, timeout=10,
                    env={**os.environ, 'CARGO_MANIFEST_DIR': str(manifest),
                         'CARGO_PKG_VERSION': '0.6.0-alpha.1'})
            output = run(app)
            self.assertIn(f'0.6.0-alpha.1 ({commit})', output)
            source.write_text('changed')
            self.assertIn(f'0.6.0-alpha.1 ({commit}, dirty)', run(app))
            linked = root / 'linked'
            git('worktree', 'add', '--detach', str(linked), commit)
            linked_output = run(linked / 'apps/framerize')
            self.assertIn(f'0.6.0-alpha.1 ({commit})', linked_output)
            head = subprocess.check_output(['git', '-C', str(linked), 'rev-parse',
                '--path-format=absolute', '--git-path', 'HEAD'], text=True).strip()
            self.assertIn('cargo:rerun-if-changed=' + head, linked_output)
