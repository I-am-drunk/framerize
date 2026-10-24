"""Source-only helpers. Importing this module never executes a phase."""
from pathlib import Path
import hashlib, json

ROOT = Path(__file__).resolve().parent.parent
REPO = Path('/home/tjninja/framerize-browserless')
TEMPLATE = ROOT / 'native-resident-api-small-r2'
CACHE = Path('/home/tjninja/framerize/target/debug/deps')
MIB = 1024 ** 2
ENROLL = 'native-enrollment-build-r1'
MAIN = ('native-standalone-contract-build-r1', 'send-main-compile')
# phase: crate, mode, cap MiB, direct project prerequisites
PHASES = {
    'core-compile': ('framer-core', 'rlib', 512, ()),
    'host-compile': ('framer-host', 'rlib', 640, ('core',)),
    'api-compile': ('framer-api', 'rlib', 768, ('core', 'host')),
    'server-compile': ('framer-server', 'rlib', 640, ('core',)),
    'resident-check-compile': ('framerize', 'metadata', 384, ('core', 'host', 'api', 'server')),
    'resident-tests-compile': ('framerize', 'test', 640, ('core', 'host', 'api', 'server')),
    'resident-run': ('framerize', 'run', 256, ()),
    'server-tests-compile': ('framer-server', 'test', 768, ('core',)),
    'server-run': ('framer-server', 'run', 256, ()),
    'store-compile': ('framer-store', 'rlib', 512, ('core',)),
    'main-check-compile': ('framerize', 'metadata', 512, ('core', 'host', 'api', 'server', 'store')),
}

def read(path):
    return json.loads(Path(path).read_text())

def sha(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()

def write(path, value):
    with Path(path).open('x') as stream:
        json.dump(value, stream, indent=2); stream.write('\n')

def recipe(phase):
    if phase.startswith('resident-') or phase == 'main-check-compile':
        return MAIN
    if phase == 'api-compile':
        return ('native-resident-api-small-r2', 'send-api-tests-compile')
    if phase == 'host-compile':
        return ('native-resident-rpc-r2', 'cancel-host-tests-compile')
    if phase == 'server-compile':
        return ('native-refusal-handoff-r1/build', 'enrollment-server-compile')
    return (ENROLL, 'enrollment-' + phase)

def upstream(b, name, pins):
    phase = name + '-compile'
    proof = b / ('result-' + phase + '.json')
    result = read(proof)
    assert result['status'] == 'pass' and result['phase'] == phase
    assert result['source_snapshot_sha256'] == sha(b / 'source-binding.json')
    contract = b / ('contract-' + phase + '-r1.json')
    manifest = b / ('inputs-' + phase + '-r1.json')
    assert result['contract_sha256'] == sha(contract)
    assert result['manifest_sha256'] == sha(manifest)
    pins.update({str(p): sha(p) for p in (proof, contract, manifest)})
    for p, digest in result['output_sha256'].items():
        assert sha(p) == digest, 'changed upstream output: ' + p
        pins[p] = digest
    return result

def artifact(b, name):
    return b / (name + '-compile-r1') / ('libframer_' + name + '.rlib')
