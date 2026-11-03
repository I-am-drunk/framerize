"""Copy an installed browser and disable redundant debugger body retention."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import zipfile
from .files import link_or_copy

PATCHES = Path(__file__).with_name("patches")
MODULE = 'chrome/juggler/content/NetworkObserver.js'
SOURCE = 'cd548efd53a6aff21107a08005adda53bb6b2d48474209712c87e6cb27e9e768'
PATCHED = '0d873f1121c3e56f8a484828c7690cd87cfb20c7c10b2c2176d8b1884734fe23'
PROTOCOL = 'chrome/juggler/content/protocol/Protocol.js'
PROTOCOL_SOURCE = 'd7bad1be5746cf6e71774963bc0e9d859f8edc4a003a0705fefb4d525034c9ec'
PROTOCOL_LEGACY = '6728608b522c6442c52e9a21cc980f43bce9daeb3e7e4fe0c63f995a9eff0aec'
PROTOCOL_PATCHED = '2c93c984f60ac64711c941aef11cf188428699dca3295498fb9a4378fb43cdfe'
CONTEXTS = 'chrome/juggler/content/TargetRegistry.js'
CONTEXTS_SOURCE = '85c0c0711648fff198c877896c28949968c93a376293aab0af82de710b80fdbd'
CONTEXTS_LEGACY = '033e67fd5a1064794f205682f5e384af77074c76bfe87ac6539430f86aa5eadb'
CONTEXTS_PATCHED = '570dc60767f79f7c35570d793cb047a8691428bf1e2724ffe129fbee317b7253'


def patch_contexts(data, directory):
    if hashlib.sha256(data).hexdigest() != CONTEXTS_SOURCE:
        raise ValueError('unsupported browser context owner; qualify this version first')
    target = directory / 'TargetRegistry.js'
    target.write_bytes(data)
    subprocess.run(['patch', '--batch', '--fuzz=0', str(target),
                    str(PATCHES / 'context_retention.patch')],
                   check=True, capture_output=True, timeout=30)
    subprocess.run(['patch', '--batch', '--fuzz=0', str(target),
                    str(PATCHES / 'page_creation.patch')],
                   check=True, capture_output=True, timeout=30)
    patched = target.read_bytes()
    guard = Path(__file__).with_name('page_creation_guard.js').read_bytes()
    patched = patched.replace(b'const helper = new Helper();',
        b'const helper = new Helper();\n\n' + guard)
    if hashlib.sha256(patched).hexdigest() != CONTEXTS_PATCHED:
        raise ValueError('browser context cleanup failed its integrity check')
    return patched


def patch_protocol(data):
    if hashlib.sha256(data).hexdigest() != PROTOCOL_SOURCE:
        raise ValueError('unsupported browser protocol; qualify this version first')
    before = b"    'requestFinished': {\n      requestId: t.String,"
    after = b"    'requestFinished': {\n      isRedirected: t.Optional(t.Boolean),\n      requestId: t.String,"
    result = data.replace(before, after)
    before = b"    'requestWillBeSent': {\n"
    result = result.replace(before, before + b'      framerizeTurnId: t.Optional(t.String),\n')
    if hashlib.sha256(result).hexdigest() != PROTOCOL_PATCHED:
        raise ValueError('browser protocol patch failed its integrity check')
    return result


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def patch_module(data, directory):
    if hashlib.sha256(data).hexdigest() != SOURCE:
        raise ValueError('unsupported browser network module; qualify this version first')
    module = directory / 'NetworkObserver.js'
    module.write_bytes(data)
    for name in ['network_capture.patch', 'network_lifecycle.patch', 'network_identity.patch']:
        subprocess.run(['patch', '--batch', '--fuzz=0', str(module),
                        str(PATCHES / name)],
                       check=True, capture_output=True, timeout=30)
    patched = module.read_bytes()
    if hashlib.sha256(patched).hexdigest() != PATCHED:
        raise ValueError('browser network patch does not match its qualified hash')
    return patched


def prepare(source, output):
    source, output = source.resolve(strict=True), output.absolute()
    if output.exists() or output.is_symlink():
        raise ValueError('output already exists; prepared browsers are immutable')
    if output.is_relative_to(source):
        raise ValueError('output must be outside the installed browser')
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=output.parent) as temporary:
        root = Path(temporary)
        with zipfile.ZipFile(source / 'omni.ja') as original:
            patched = patch_module(original.read(MODULE), root)
            protocol = patch_protocol(original.read(PROTOCOL))
            contexts = patch_contexts(original.read(CONTEXTS), root)
            tree = root / 'browser'
            shutil.copytree(source, tree, copy_function=link_or_copy, symlinks=True)
            archive = root / 'patched-omni.ja'
            with zipfile.ZipFile(archive, 'w') as target:
                target.comment = original.comment
                for entry in original.infolist():
                    replacements = {MODULE: patched, PROTOCOL: protocol, CONTEXTS: contexts}
                    data = replacements.get(entry.filename)
                    if data is None:
                        data = original.read(entry)
                    target.writestr(entry, data)
        # Replace only the copied directory entry, never the linked source file.
        archive.replace(tree / 'omni.ja')
        manifest = {'patch_version': 13, 'source_directory': str(source),
            'source_module_sha256': SOURCE, 'patched_module_sha256': PATCHED,
            'source_archive_sha256': digest(source / 'omni.ja'),
            'patched_archive_sha256': digest(tree / 'omni.ja'),
            'preference': 'framerize.captureResponseBodies',
            'request_preference': 'framerize.captureRequestBodies',
            'identity_preference': 'framerize.captureRequestIdentity',
            'redirect_preference': 'framerize.reportActualRedirects',
            'protocol_module_sha256': PROTOCOL_PATCHED,
            'context_module_sha256': CONTEXTS_PATCHED}
        (tree / 'framerize-memory-patch.json').write_text(json.dumps(manifest, indent=2)+'\n')
        tree.rename(output)
    return manifest
