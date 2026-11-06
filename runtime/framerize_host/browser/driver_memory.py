"""Prepare a pinned Playwright driver without changing the installed package."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import tempfile
from .files import apply_unified_patch, link_or_copy

MODULE = 'package/lib/coreBundle.js'
SOURCE = '3258d1cf334c6afc95f22aa9c292436cb976b391e0437f1359c83b84f0cb9d66'
PATCHED = '9b755edf4bae8bfc061138733ae7a2ae805aa33eac67bf5d293a348d109a2b7b'


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def patch_driver(source, target):
    if digest(source) != SOURCE:
        raise ValueError('unsupported Playwright driver; qualify its memory cleanup before use')
    shutil.copy2(source, target)
    for name in ('playwright_memory.patch', 'driver_identity.patch'):
        patch = Path(__file__).with_name('patches') / name
        apply_unified_patch(target, patch)
    if digest(target) != PATCHED:
        raise ValueError('Playwright memory patch failed its integrity check')


def prepare(source, output):
    source, output = source.resolve(strict=True), output.absolute()
    if output.exists() or output.is_symlink():
        raise ValueError('driver output already exists; refusing to replace it')
    if output.is_relative_to(source):
        raise ValueError('driver output must be outside the installed driver')
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=output.parent) as temporary:
        temporary = Path(temporary)
        patched = temporary / 'coreBundle.js'
        patch_driver(source / MODULE, patched)
        tree = temporary / 'driver'
        shutil.copytree(source, tree, copy_function=link_or_copy, symlinks=True)
        patched.replace(tree / MODULE)
        files = {name: digest(tree / name) for name in (
            MODULE, 'package/cli.js', 'package/package.json')}
        (tree / 'framerize-driver-memory.json').write_text(json.dumps({
            'source_module_sha256': SOURCE, 'files': files}, indent=2)+'\n')
        tree.rename(output)
    return output


def install(root):
    from playwright._impl import _transport
    current = _transport.compute_driver_executable
    if getattr(current, '_framerize_memory', False):
        return
    node, cli = current()
    source = Path(cli).resolve().parent.parent
    if digest(source / MODULE) != SOURCE:
        raise RuntimeError('Installed Playwright driver does not match the qualified memory patch')
    target = Path(root) / '.runtime' / ('driver-' + PATCHED[:16])
    if not target.exists():
        prepare(source, target)
    expected = {MODULE: PATCHED,
        'package/cli.js': digest(source / 'package/cli.js'),
        'package/package.json': digest(source / 'package/package.json')}
    if any(digest(target / name) != value for name, value in expected.items()):
        raise RuntimeError('Prepared Playwright driver failed its memory-patch integrity check')
    def executable():
        return node, str(target / 'package/cli.js')
    executable._framerize_memory = True
    _transport.compute_driver_executable = executable
    os.environ['FRAMERIZE_DISPOSE_EVICTED_CONSOLE'] = '1'
    os.environ['FRAMERIZE_LINEAR_PIPE_DECODER'] = '1'
    os.environ['FRAMERIZE_BATCH_CONSOLE_DISPOSAL'] = '0'
