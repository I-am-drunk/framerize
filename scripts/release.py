"""Stage complete releases and activate an already migrated user service."""
import argparse
import hashlib
import json
from pathlib import Path
import tarfile
import tempfile


def sha256(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def verify(root):
    files = json.loads((root / 'manifest.json').read_text())
    paths = list(root.rglob('*'))
    if any(path.is_symlink() for path in paths):
        raise ValueError('release contains a symbolic link')
    actual = {path.relative_to(root).as_posix() for path in paths
              if path.is_file() and path != root / 'manifest.json'}
    if type(files) is not dict or set(files) != actual:
        raise ValueError('release file inventory differs')
    if any(sha256(root / name) != digest for name, digest in files.items()):
        raise ValueError('release file checksum differs')
    info = json.loads((root / 'build-info.json').read_text())
    app = info['application']
    if info['format'] != 1 or app not in ('framerize', 'framerize-native'):
        raise ValueError('unsupported application package')
    required = {f'bin/{app}'}
    if app == 'framerize':
        required.update(('runtime/host.py', 'runtime/runtime-manifest.json', 'web/index.html'))
    else:
        required.update(('runtime/native.py', 'runtime/runtime-manifest.json'))
        if info.get('worker_bundle_format') == 1:
            required.update(('runtime/worker/worker-config.json', 'runtime/worker/worker.mjs',
                             'runtime/worker/bin/node', 'runtime/worker/bundle-manifest.json'))
    if not required <= actual:
        raise ValueError('incomplete application package')
    return info


def stage(archive, expected, releases):
    if sha256(archive) != expected:
        raise ValueError('archive checksum differs')
    releases.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='.stage-', dir=releases) as temporary:
        temporary = Path(temporary)
        with tarfile.open(archive, 'r:gz') as bundle:
            entries = bundle.getmembers()
            if not entries or len(entries) > 10000 or sum(e.size for e in entries) > 2 * 1024**3:
                raise ValueError('archive exceeds release limits')
            for entry in entries:
                path = Path(entry.name)
                if path.is_absolute() or '..' in path.parts or not (entry.isdir() or entry.isfile()):
                    raise ValueError('unsafe release archive entry')
            bundle.extractall(temporary, filter='data')
        roots = list(temporary.iterdir())
        if len(roots) != 1 or not roots[0].is_dir():
            raise ValueError('archive must contain one release directory')
        root = roots[0]
        verify(root)
        destination = releases / root.name
        if destination.exists() or destination.is_symlink():
            raise ValueError('release already exists; refusing to overwrite it')
        root.rename(destination)
        return destination


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    staging = commands.add_parser('stage', help='verify and unpack an immutable release')
    staging.add_argument('archive', type=Path)
    staging.add_argument('--sha256', required=True, help='checksum supplied with the build artifact')
    staging.add_argument('--releases', type=Path,
                         default=Path.home() / '.local/share/framerize/releases')
    activation = commands.add_parser('activate', help='switch a managed service with rollback')
    activation.add_argument('release', type=Path)
    verification = commands.add_parser('verify', help='check installed files without starting or changing anything')
    verification.add_argument('release', type=Path)
    args = parser.parse_args()
    try:
        if args.command == 'verify':
            print(json.dumps(verify(args.release.resolve())))
        elif args.command == 'activate':
            from release_activation import run
            print(json.dumps(run(args.release)))
        else:
            print(stage(args.archive.resolve(), args.sha256, args.releases.resolve()))
    except Exception as error:
        parser.exit(1, f'{error}\n')
    except KeyboardInterrupt:
        parser.exit(130, 'activation interrupted; rerun to recover any pending update\n')
