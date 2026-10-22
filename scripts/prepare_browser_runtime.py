"""Prepare browser host sources in a new directory; never install or start them."""
import argparse
import ast
import hashlib
import json
import shutil
from pathlib import Path


def prepare(destination):
    scripts = Path(__file__).resolve().parent
    source = scripts / 'cooldown_support'
    names = ('__init__.py', 'startup_state.py', 'inventory.py', 'ledger.py',
             'private_state.py')
    files = {name: source / name for name in names}
    files['native_device.py'] = scripts / 'native_device.py'
    pins = {name: hashlib.sha256(path.read_bytes()).hexdigest()
            for name, path in files.items()}
    manifest = (json.dumps(pins, sort_keys=True, indent=2) + '\n').encode()
    digest = hashlib.sha256(manifest).hexdigest()
    support_name = 'cooldown-support-' + digest[:16]
    hook = (source / 'startup_hook.txt').read_text()
    hook = hook.replace('MANIFEST_SHA256', digest)
    hook = hook.replace('SUPPORT_DIRECTORY', support_name)
    raw = (scripts / 'camoufox_host.py').read_text()
    entry = '    asyncio.run(Host().loop())\n'
    if raw.count(entry) != 1:
        raise ValueError('expected exactly one browser host entry point')
    generated = raw.replace(entry, hook + entry)
    ast.parse(generated)
    destination.mkdir(parents=True, exist_ok=False)
    support = destination / support_name
    support.mkdir()
    for name, path in files.items():
        shutil.copyfile(path, support / name)
    (support / 'runtime-sources.json').write_bytes(manifest)
    host = destination / 'camoufox_host.py'
    host.write_text(generated)
    return {'sidecar_sha256': hashlib.sha256(host.read_bytes()).hexdigest(),
            'support_sha256': digest}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path, help='new output directory')
    args = parser.parse_args()
    print(json.dumps(prepare(args.output), sort_keys=True))
