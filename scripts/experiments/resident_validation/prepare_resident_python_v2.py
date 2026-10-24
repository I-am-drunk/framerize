"""Freeze inert validation sources only. Does not launch tests or read live state."""
from pathlib import Path
import argparse, ast, hashlib, json, re, shutil

root = Path(__file__).resolve().parent
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--directory', required=True, help='Fresh bundle name under this preparer directory')
args = parser.parse_args()
if not re.fullmatch(r'native-resident-python-v2-[a-z0-9][a-z0-9-]{0,79}', args.directory):
    parser.error('--directory must be a simple native-resident-python-v2-* bundle name')
old, new = root / 'native-resident-operation-r1', root / args.directory
repo = Path('/home/tjninja/framerize-browserless')
expected_counts = {
    'resident_authorization_tests': 20, 'resident_host_dispatch_tests': 5,
    'resident_crossover_tests': 29, 'resident_bootstrap_tests': 9,
    'resident_http200_provenance_tests': 21, 'resident_host_activity_tests': 31,
    'resident_crossover_provenance_tests': 5, 'resident_crossover_refusal_tests': 5,
    'resident_cooldown_effects_tests': 5,
}
modules = list(expected_counts)
expected_total = 130
assert sum(expected_counts.values()) == expected_total
javascript = ['resident_authorization_js_tests.cjs', 'resident_wrapper_provenance_tests.cjs',
              'resident_crossover_js_tests.cjs']
paths = ['scripts/' + name + '.py' for name in ('camoufox_host', 'resident_activity',
         'contextual_authorization', 'resident_authorization', 'resident_bootstrap', 'observed_sidecar')]
paths += ['reference/fixtures/' + name + '.py' for name in modules]
paths += ['reference/fixtures/' + name for name in javascript]
cases = []
new.mkdir(mode=0o700)
sha = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
before = {name: sha(repo / name) for name in paths}
for name in paths:
    target = new / 'source' / name
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(repo / name, target)
    assert sha(target) == before[name], 'source changed while copying: ' + name
    if name.endswith('_tests.py'):
        tree = ast.parse(target.read_text(), filename=str(target))
        discovered = [target.stem + '.' + cls.name + '.' + n.name
                      for cls in tree.body if isinstance(cls, ast.ClassDef)
                      for n in cls.body if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef))
                      and n.name.startswith('test_')]
        assert len(discovered) == len(set(discovered)) == expected_counts[target.stem], target.stem
        cases.extend(discovered)
cases.sort()
assert len(cases) == len(set(cases)) == expected_total
run = (old / 'run.py').read_text()
start, end = run.index('suite ='), run.index('result = unittest')
run = run[:start] + '''import importlib
sys.path.insert(0, str(B / 'source/reference/fixtures'))
suite = unittest.TestSuite(unittest.defaultTestLoader.loadTestsFromModule(
    importlib.import_module(name)) for name in MODULES)
expected = json.loads((B / 'cases.json').read_text())
def case_ids(suite):
    for item in suite:
        if isinstance(item, unittest.TestSuite):
            yield from case_ids(item)
        else:
            yield item.id()
discovered = list(case_ids(suite))
assert len(expected) == len(set(expected)) == EXPECTED_TOTAL
assert len(discovered) == len(set(discovered)) == EXPECTED_TOTAL
assert sorted(discovered) == expected, 'discovered test identities differ from frozen AST inventory'
'''.replace('MODULES', repr(modules)).replace('EXPECTED_TOTAL', str(expected_total)) + run[end:]
assert run.count('result.testsRun == 25') == 1, 'runner template changed'
run = run.replace('result.testsRun == 25', 'result.testsRun == ' + str(expected_total))
run = run.replace("('resident_authorization_js_tests.cjs', 'resident_wrapper_provenance_tests.cjs')",
                  repr(tuple(javascript)))
ast.parse(run)
(new / 'run.py').write_text(run)
launcher = (old / 'launch.py').read_text()
assert launcher.count("unit = 'framerize-resident-operation-r1'") == 1
launcher = launcher.replace("unit = 'framerize-resident-operation-r1'",
                            'unit = ' + repr('framerize-' + args.directory.removeprefix('native-')))
ast.parse(launcher)
(new / 'launch.py').write_text(launcher)
(new / 'cases.json').write_text(json.dumps(cases, indent=2))
assert before == {name: sha(repo / name) for name in paths}, 'concurrent source change'
(new / 'source-bindings.json').write_text(json.dumps(before, indent=2))
pins = {str(p): sha(p) for p in new.rglob('*') if p.is_file()}
(new / 'inputs.json').write_text(json.dumps(pins, indent=2))
print(json.dumps(dict(prepared=str(new), python_cases=len(cases), js_fixtures=3,
                     execution='NOT_STARTED')))
