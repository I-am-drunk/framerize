"""Run the host's actual decorated warm/mint strings in an inert JavaScript VM."""
import ast
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
from resident_authorization import observe_warm_identity


def scripts():
    tree = ast.parse((ROOT / 'scripts/camoufox_host.py').read_text())
    values = {n.targets[0].id: n.value for n in tree.body
              if isinstance(n, ast.Assign) and isinstance(n.targets[0], ast.Name)}
    namespace = {'CASTLE_CHUNK': ast.literal_eval(values['CASTLE_CHUNK'])}
    result = {}
    for name in ('WARM_JS', 'MINT_JS'):
        expression = ast.Expression(values[name])
        raw = eval(compile(expression, '<host-literal>', 'eval'),
                   {'__builtins__': {}}, namespace)
        result[name] = observe_warm_identity(raw)
    return result


if __name__ == '__main__':
    command = ['/usr/bin/node', '--experimental-vm-modules',
               str(Path(__file__).with_suffix('.cjs'))]
    result = subprocess.run(command, input=json.dumps(scripts()), text=True,
                            timeout=10, check=False)
    raise SystemExit(result.returncode)
