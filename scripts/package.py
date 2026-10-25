"""Package a built application with the runtime and UI from the same checkout."""
import argparse
import hashlib
import json
from pathlib import Path
import platform
import shutil
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def package(app, output, target_dir, allow_dirty=False):
    commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    dirty = bool(subprocess.check_output(
        ['git', 'status', '--porcelain', '--untracked-files=normal'], cwd=ROOT))
    if dirty and not allow_dirty:
        raise ValueError('commit the source first, or use --allow-dirty for a local development package')
    profile = target_dir.name
    development = dirty or profile != 'release'
    if profile != 'release' and not allow_dirty:
        raise ValueError('use a release build, or --allow-dirty for a local development package')
    system = f'{platform.system().lower()}-{platform.machine()}'
    name = f'{app}-{system}-{commit}' + ('-development' if development else '')
    binary = target_dir / app
    if not binary.is_file():
        raise ValueError(f'build the {app} target first: {binary}')
    output.mkdir(parents=True, exist_ok=True)
    archive = output / f'{name}.tar.gz'
    with tempfile.TemporaryDirectory() as temporary:
        tree = Path(temporary) / name
        (tree / 'bin').mkdir(parents=True)
        shutil.copy2(binary, tree / 'bin' / app)
        if app == 'framerize':
            service = tree / 'share/systemd/user'
            service.mkdir(parents=True)
            shutil.copy2(ROOT / 'packaging/systemd/framerize.service', service)
            if not (ROOT / 'web/dist/index.html').is_file():
                raise ValueError('build the UI first: npm --prefix web ci && npm --prefix web run build')
            shutil.copytree(ROOT / 'web/dist', tree / 'web')
            source = ROOT / 'runtime'
            shutil.copytree(source / 'framerize_host', tree / 'runtime/framerize_host',
                            ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
            for entry in ['host.py', 'requirements.txt']:
                shutil.copy2(source / entry, tree / 'runtime' / entry)
            runtime = tree / 'runtime'
            for path in sorted(runtime.rglob('*'), reverse=True):
                if path.is_dir() and not any(path.iterdir()):
                    path.rmdir()
            pins = {path.relative_to(runtime).as_posix():
                    hashlib.sha256(path.read_bytes()).hexdigest()
                    for path in sorted(runtime.rglob('*')) if path.is_file()}
            (runtime / 'runtime-manifest.json').write_text(
                json.dumps(pins, sort_keys=True, indent=2) + '\n')
        info = {'format': 1, 'application': app, 'commit': commit, 'platform': system, 'source_dirty': dirty,
                'build_profile': profile, 'development': development,
                'rustc': subprocess.check_output(['rustc', '--version'], text=True).strip(),
                'capability': 'browser-serving' if app == 'framerize' else 'configuration-structure-only'}
        if app == 'framerize':
            info['rate_history_schema'] = 2
        (tree / 'build-info.json').write_text(json.dumps(info, indent=2) + '\n')
        files = {path.relative_to(tree).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
                 for path in sorted(tree.rglob('*')) if path.is_file()}
        (tree / 'manifest.json').write_text(json.dumps(files, sort_keys=True, indent=2) + '\n')
        with tarfile.open(archive, 'w:gz') as bundle:
            bundle.add(tree, arcname=name)
    with archive.open('rb') as stream:
        digest = hashlib.file_digest(stream, 'sha256').hexdigest()
    archive.with_suffix(archive.suffix + '.sha256').write_text(f'{digest}  {archive.name}\n')
    return archive


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('app', choices=['framerize', 'framerize-native'])
    parser.add_argument('--output', type=Path, default=ROOT / 'dist')
    parser.add_argument('--target-dir', type=Path, default=ROOT / 'target/release')
    parser.add_argument('--allow-dirty', action='store_true', help='mark a local development package')
    args = parser.parse_args()
    print(package(args.app, args.output.resolve(), args.target_dir.resolve(), args.allow_dirty))
