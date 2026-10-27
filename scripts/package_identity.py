"""Bind the packaged executable's own version to the source being packaged."""
import re
import subprocess
import tomllib


def identify(root, app, binary, commit, dirty):
    manifest = tomllib.loads((root / 'apps' / app / 'Cargo.toml').read_text())
    version = manifest['package']['version']
    if not re.fullmatch(r'\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?', version):
        raise ValueError('invalid application version')
    expected = f'{app} {version} ({commit}' + (', dirty)' if dirty else ')')
    actual = subprocess.check_output([str(binary), '--version'], text=True, timeout=10).strip()
    if actual != expected:
        raise ValueError('binary identity differs from source; rebuild this application')
    return {'version': version, 'version_output': actual,
            'release_tag': 'v' + version,
            'release_url': 'https://github.com/I-am-drunk/framerize/releases/tag/v' + version}
