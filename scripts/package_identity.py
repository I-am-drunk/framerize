"""Bind the packaged executable's own version to the source being packaged."""
import json
import re
import subprocess
import tomllib
from release_checkpoint_history import formats


def identify(root, app, binary, commit, dirty):
    manifest = tomllib.loads((root / 'apps' / app / 'Cargo.toml').read_text())
    version = manifest['package']['version']
    if not re.fullmatch(r'\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?', version):
        raise ValueError('invalid application version')
    expected = f'{app} {version} ({commit}' + (', dirty)' if dirty else ')')
    actual = subprocess.check_output([str(binary), '--version'], text=True, timeout=10).strip()
    if actual != expected:
        raise ValueError('binary identity differs from source; rebuild this application')
    info = {'version': version, 'version_output': actual,
            'release_tag': 'v' + version,
            'release_url': 'https://github.com/I-am-drunk/framerize/releases/tag/v' + version}
    if app == 'framerize':
        try:
            capabilities = json.loads(subprocess.check_output(
                [str(binary), 'checkpoint-history'], text=True, timeout=10))
            formats({'checkpoint_history': capabilities})
        except (ValueError, subprocess.SubprocessError) as error:
            raise ValueError('binary checkpoint capabilities are unavailable or invalid') from error
        info['checkpoint_history'] = capabilities
        info['build_variant'] = ('legacy-checkpoint-writes'
                                 if capabilities['write_version'] == 1 else 'default')
    return info
