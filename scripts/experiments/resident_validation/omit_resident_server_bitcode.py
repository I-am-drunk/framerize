"""Prepare server rlib without unused LLVM bitcode; resource limits unchanged."""
from pathlib import Path
import hashlib, json
root = Path(__file__).resolve().parent
b = root / 'native-resident-worker-validation-r5'
p = b / 'contract-server-compile-r1.json'
m = b / 'inputs-server-compile-r1.json'
assert not (b / 'server-compile-r1').exists()
def sha(p):
    return hashlib.sha256(p.read_bytes()).hexdigest()
c = json.loads(p.read_text())
argv = c['stages'][0]['argv']
assert '-Cembed-bitcode=no' not in argv
argv.insert(argv.index('-Copt-level=0') + 1, '-Cembed-bitcode=no')
c['build_adjustment'] = 'Omit unused LLVM bitcode for non-LTO rlib; source and resource limits unchanged.'
source = Path(__file__).resolve()
c['prepared_inputs'][str(source)] = sha(source)
c['required_inputs'].append(str(source))
p.write_text(json.dumps(c, indent=2) + '\n')
m.write_text(json.dumps({'sha256': c['prepared_inputs'] | {str(p): sha(p)}}, indent=2) + '\n')
print('Prepared server bitcode omission; caps unchanged.')
