"""Prepare the one API retry without unused LLVM bitcode; limits unchanged."""
from pathlib import Path
import hashlib, json
root = Path(__file__).resolve().parent
b = root / 'native-resident-worker-validation-r5'
p = b / 'contract-api-compile-r1.json'
m = b / 'inputs-api-compile-r1.json'
assert not (b / 'api-compile-r1').exists()
def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()
c = json.loads(p.read_text())
argv = c['stages'][0]['argv']
assert '-Cembed-bitcode=no' not in argv
argv.insert(argv.index('-Copt-level=0') + 1, '-Cembed-bitcode=no')
c['build_adjustment'] = 'Omit unused LLVM bitcode after r4 SIGXFSZ at 32 MiB; unchanged source and resource limits.'
for source in (Path(__file__).resolve(), root / 'reuse_resident_r4_results.py'):
    c['prepared_inputs'][str(source)] = sha(source)
    c['required_inputs'].append(str(source))
p.write_text(json.dumps(c, indent=2) + '\n')
pins = c['prepared_inputs'] | {str(p): sha(p)}
m.write_text(json.dumps({'sha256': pins}, indent=2) + '\n')
print('Prepared explicit bitcode-only API build adjustment; caps unchanged.')
