"""Run only the explicitly bound inert recovery cases; refuse skips."""
import json, sys, unittest
from pathlib import Path
HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
names = json.loads((HERE/'selection.json').read_text())['names']
assert names and len(names) == len(set(names))
suite = unittest.defaultTestLoader.loadTestsFromNames(names)
def ids(group):
    for case in group:
        if isinstance(case, unittest.TestSuite):
            yield from ids(case)
        else:
            yield case.id()
assert list(ids(suite)) == names, 'selected test inventory changed'
result = unittest.TextTestRunner(verbosity=2).run(suite)
print(json.dumps(dict(selected=len(names), run=result.testsRun,
                      failures=len(result.failures), errors=len(result.errors),
                      skipped=len(result.skipped))))
raise SystemExit(0 if result.wasSuccessful() and not result.skipped
                 and result.testsRun == len(names) else 1)
