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
from package_identity import identify

ROOT = Path(__file__).resolve().parents[1]


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def copy_worker(source, destination):
    if not (source / 'bundle-manifest.json').is_file():
        raise ValueError('build the native worker first: node scripts/offline_worker/bundle.mjs --output target/native-worker')
    shutil.copytree(source, destination, symlinks=True)
    paths = list(destination.rglob('*'))
    if any(path.is_symlink() for path in paths):
        raise ValueError('worker bundle contains a symbolic link')
    files = json.loads((destination / 'bundle-manifest.json').read_text())
    stamp = destination / 'bundle-source.json'
    if not stamp.is_file():
        raise ValueError('worker bundle has no source stamp: rebuild scripts/offline_worker/bundle.mjs')
    head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    if json.loads(stamp.read_text()).get('commit') != head:
        raise ValueError('worker bundle is stale: rebuild scripts/offline_worker/bundle.mjs at HEAD')
    actual = {p.relative_to(destination).as_posix() for p in paths
              if p.is_file() and p.name != 'bundle-manifest.json'}
    if type(files) is not dict or actual != set(files):
        raise ValueError('worker bundle inventory changed')
    if not {'bin/node', 'worker.mjs', 'worker-config.json'} <= actual:
        raise ValueError('incomplete worker bundle')
    for name, expected in files.items():
        with (destination / name).open('rb') as stream:
            if hashlib.file_digest(stream, 'sha256').hexdigest() != expected:
                raise ValueError('worker bundle file changed: ' + name)


def copy_runtime(app, tree, worker_bundle=None):
    source, runtime = ROOT / 'runtime', tree / 'runtime'
    browser = app == 'framerize'
    modules = ('framerize_accounts', 'framerize_runtime',
               'framerize_host' if browser else 'framerize_native')
    for module in modules:
        shutil.copytree(source / module, runtime / module,
                        ignore=shutil.ignore_patterns('__pycache__', '*.pyc'))
    for entry in (('host.py', 'requirements.txt') if browser else ('native.py',)):
        shutil.copy2(source / entry, runtime / entry)
    if not browser:
        copy_worker(worker_bundle or ROOT / 'target/native-worker', runtime / 'worker')
    for path in sorted(runtime.rglob('*'), reverse=True):
        if path.is_dir() and not any(path.iterdir()):
            path.rmdir()
    pins = {path.relative_to(runtime).as_posix(): digest(path)
            for path in sorted(runtime.rglob('*')) if path.is_file()}
    (runtime / 'runtime-manifest.json').write_text(json.dumps(pins, sort_keys=True, indent=2) + '\n')


def package(app, output, target_dir, allow_dirty=False, worker_bundle=None):
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
    identity = identify(ROOT, app, binary, commit, dirty) if app == "framerize" else {}
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
        copy_runtime(app, tree, worker_bundle)
        info = {'format': 1, 'application': app, 'commit': commit, 'platform': system, 'source_dirty': dirty,
                'build_profile': profile, 'development': development,
                'rustc': subprocess.check_output(['rustc', '--version'], text=True).strip(),
                'capability': 'browser-serving' if app == 'framerize' else 'experimental-native-serving'}
        info.update(identity)
        if app == 'framerize':
            info['rate_history_schema'] = 2
            info['activation_protocol'] = 1
        else:
            info['worker_bundle_format'] = 1
        (tree / 'build-info.json').write_text(json.dumps(info, indent=2) + '\n')
        files = {path.relative_to(tree).as_posix(): digest(path)
                 for path in sorted(tree.rglob('*')) if path.is_file()}
        (tree / 'manifest.json').write_text(json.dumps(files, sort_keys=True, indent=2) + '\n')
        with tarfile.open(archive, 'w:gz') as bundle:
            bundle.add(tree, arcname=name)
    with archive.open('rb') as stream:
        archive_digest = hashlib.file_digest(stream, 'sha256').hexdigest()
    archive.with_suffix(archive.suffix + '.sha256').write_text(f'{archive_digest}  {archive.name}\n')
    return archive


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('app', choices=['framerize', 'framerize-native'])
    parser.add_argument('--output', type=Path, default=ROOT / 'dist')
    parser.add_argument('--target-dir', type=Path, default=ROOT / 'target/release')
    parser.add_argument('--allow-dirty', action='store_true', help='mark a local development package')
    parser.add_argument('--worker-bundle', type=Path,
                        help='prepared native worker directory (default: target/native-worker)')
    args = parser.parse_args()
    print(package(args.app, args.output.resolve(), args.target_dir.resolve(),
                  args.allow_dirty, args.worker_bundle))
