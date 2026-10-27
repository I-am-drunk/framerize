"""Validate the exact CI packages before creating an alpha release draft."""
import hashlib
import json
from pathlib import PurePosixPath
import re
import tarfile


def inspect(archive, app, commit):
    prefix = f'{app}-linux-x86_64-{commit}'
    expected_name = prefix + '.tar.gz'
    if archive.name != expected_name:
        raise ValueError('unexpected artifact name')
    with archive.open('rb') as source:
        digest = hashlib.file_digest(source, 'sha256').hexdigest()
    if archive.with_suffix('.gz.sha256').read_text().strip() != f'{digest}  {expected_name}':
        raise ValueError('artifact checksum differs')
    files, metadata = {}, {}
    total = 0
    with tarfile.open(archive, 'r:gz') as bundle:
        for index, member in enumerate(bundle):
            path = PurePosixPath(member.name)
            if index > 10000 or '..' in path.parts or path.is_absolute():
                raise ValueError('unsafe artifact inventory')
            if not path.parts or path.parts[0] != prefix:
                raise ValueError('unexpected artifact root')
            if member.isdir():
                continue
            if not member.isfile():
                raise ValueError('artifact contains a link or special file')
            name = '/'.join(path.parts[1:])
            total += member.size
            if name in files or total > 1024 ** 3:
                raise ValueError('duplicate or oversized artifact')
            source = bundle.extractfile(member)
            files[name] = hashlib.file_digest(source, 'sha256').hexdigest()
            if name in ['build-info.json', 'manifest.json']:
                if member.size > 1024 * 1024:
                    raise ValueError('oversized metadata')
                source.seek(0)
                metadata[name] = json.load(source)
    info = metadata.get('build-info.json', {})
    version = info.get('version', '')
    if not re.fullmatch(r'\d+\.\d+\.\d+-alpha\.[1-9]\d*', version):
        raise ValueError('only numbered alpha releases are supported')
    if any(info.get(key) != value for key, value in {
            'commit': commit, 'application': app, 'source_dirty': False,
            'development': False, 'build_profile': 'release',
            'platform': 'linux-x86_64', 'release_tag': 'v' + version,
            'version_output': f'{app} {version} ({commit})'}.items()):
        raise ValueError('artifact build identity differs')
    files.pop('manifest.json', None)
    if files != metadata.get('manifest.json'):
        raise ValueError('artifact content differs from its manifest')
    if not {'bin/' + app, 'build-info.json', 'runtime/runtime-manifest.json'} <= files.keys():
        raise ValueError('incomplete application artifact')
    return {'version': version, 'tag': 'v' + version, 'commit': commit,
            'application': app, 'sha256': digest, 'file': archive.name}
