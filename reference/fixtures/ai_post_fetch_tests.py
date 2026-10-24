"""Execute the actual host job scripts with inert fetch/timer implementations."""
import ast
import json
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]


def scripts():
    tree = ast.parse((ROOT / 'scripts/camoufox_host.py').read_text())
    values = {n.targets[0].id: n.value for n in tree.body
              if isinstance(n, ast.Assign) and isinstance(n.targets[0], ast.Name)}
    namespace = {'API': ast.literal_eval(values['API'])}
    return {name: eval(compile(ast.Expression(values[name]), '<host-literal>', 'eval'),
                       {'__builtins__': {}}, namespace)
            for name in ('AI_START_JS', 'AI_POLL_JS', 'AI_CANCEL_JS')}


if __name__ == '__main__':
    result = subprocess.run(['/usr/bin/node', str(Path(__file__).with_suffix('.cjs'))],
                            input=json.dumps(scripts()), text=True, timeout=10)
    raise SystemExit(result.returncode)
