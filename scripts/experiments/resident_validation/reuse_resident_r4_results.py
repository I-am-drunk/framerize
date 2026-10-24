"""Explicit exact-artifact reuse; no compiler or fixture execution."""
from pathlib import Path
import hashlib, json, shutil, sys
root = Path(__file__).resolve().parent
old = root / 'native-resident-worker-validation-r4'
new = root / 'native-resident-worker-validation-r5'
sys.path.insert(0, str(old))
from receipt_checks import validate
def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()
def read(p):
    return json.loads(p.read_text())
assert sha(old / 'source-binding.json') == sha(new / 'source-binding.json')
for phase in ('core-compile', 'host-compile', 'server-tests-compile', 'server-run'):
    proof = old / ('result-' + phase + '.json')
    result = read(proof)
    assert validate(phase) == result
    outputs = {}
    for name, digest in result['output_sha256'].items():
        source = Path(name)
        assert sha(source) == digest
        target = new / source.relative_to(old)
        target.parent.mkdir(mode=0o700, exist_ok=True)
        shutil.copyfile(source, target)
        assert sha(target) == digest
        outputs[str(target)] = digest
    for prefix in ('contract-', 'inputs-'):
        name = prefix + phase + '-r1.json'
        shutil.copyfile(old / name, new / name)
    result['output_sha256'] = outputs
    result['reused_from'] = dict(result=str(proof), sha256=sha(proof),
        reason='Identical source binding and exact validated artifact copies; not reexecuted.')
    with (new / proof.name).open('x') as stream:
        json.dump(result, stream, indent=2)
    print('reused', phase)
