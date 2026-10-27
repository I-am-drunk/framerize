"""Exercise the real Cargo cache in disposable, dependency-free repositories."""
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


class BuildIdentityTests(unittest.TestCase):
    def test_incremental_identity_in_clone_and_linked_worktree(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            repo = root / 'repo'
            app = repo / 'apps/framerize'
            app.mkdir(parents=True)
            (repo / 'scripts').mkdir()
            for name in ['build_identity.rs', 'cargo_with_identity.py']:
                shutil.copy2(ROOT / 'scripts' / name, repo / 'scripts' / name)
            (repo / '.gitignore').write_text('/target\n')
            (repo / 'Cargo.toml').write_text('[workspace]\nmembers=["apps/framerize"]\nresolver="2"\n')
            (app / 'Cargo.toml').write_text('[package]\nname="cache-stub"\nversion="0.6.0-alpha.1"\n'
                'edition="2021"\nbuild="../../scripts/build_identity.rs"\n')
            (app / 'src').mkdir()
            (app / 'src/main.rs').write_text('fn main() { println!("{}", env!("FRAMERIZE_BUILD_VERSION")); }')
            def call(argv, cwd=repo):
                return subprocess.run(argv, cwd=cwd, text=True, capture_output=True, check=True, timeout=60)
            def git(*args, cwd=repo):
                return call(['git', '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                             '-c', 'commit.gpgsign=false', *args], cwd).stdout.strip()
            git('init', '-q')
            call(['cargo', 'generate-lockfile', '--offline'])
            git('add', '.')
            git('commit', '-qm', 'fixture')
            commit = git('rev-parse', 'HEAD')
            def build(checkout, expected, fresh=False, wrapped=True):
                command = [sys.executable, 'scripts/cargo_with_identity.py'] if wrapped else ['cargo']
                env = {k: v for k, v in os.environ.items() if not k.startswith('FRAMERIZE_SOURCE_')}
                env['CARGO_TARGET_DIR'] = str(checkout / 'target')
                result = subprocess.run([*command, 'build', '--offline', '-v'], cwd=checkout,
                                        env=env, capture_output=True, text=True, timeout=60)
                self.assertEqual(result.returncode, 0, result.stderr)
                if fresh:
                    self.assertIn('Fresh cache-stub', result.stderr)
                binary = checkout / 'target/debug/cache-stub'
                self.assertEqual(call([str(binary)], checkout).stdout.strip(), expected)
            clean = f'0.6.0-alpha.1 ({commit})'
            dirty = f'0.6.0-alpha.1 ({commit}, dirty)'
            build(repo, clean)
            build(repo, clean, fresh=True)
            git('pack-refs', '--all', '--prune')
            build(repo, clean, fresh=True)
            untracked = repo / 'untracked.txt'
            untracked.write_text('new source')
            build(repo, dirty)
            build(repo, dirty, fresh=True)
            untracked.unlink()
            build(repo, clean)
            build(repo, clean, fresh=True)
            git('commit', '--allow-empty', '-qm', 'new HEAD')
            commit = git('rev-parse', 'HEAD')
            clean = f'0.6.0-alpha.1 ({commit})'
            build(repo, clean)
            build(repo, clean, fresh=True)
            linked = root / 'linked'
            git('worktree', 'add', '--detach', str(linked), commit)
            build(linked, clean)
            build(linked, clean, fresh=True)
            source = linked / 'apps/framerize/src/main.rs'
            original = source.read_text()
            source.write_text(original + '\n// source changed\n')
            build(linked, f'0.6.0-alpha.1 ({commit}, dirty)')
            source.write_text(original)
            build(linked, clean)
            unverified = '0.6.0-alpha.1 (unverified source; build with scripts/cargo_with_identity.py)'
            build(linked, unverified, wrapped=False)
            build(linked, unverified, fresh=True, wrapped=False)
