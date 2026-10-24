"""Adapt the proven capped offline runner, with explicit narrow source closure."""
import ast, re
from common import *

HELPERS = {
    'run_validation.py': 'fe4da47662b16319c66d64334cb127ebaab1980fb631aa3c79272ebaeef75efc',
    'receipt_checks.py': '1830b0ca9fe255d43127f849dffbdb2e4e2df280267368f45010db91ca9f6353',
    'log_rows.py': 'a28926359dc93a9291fb6131e574e295ad9fc2b1cb3919a60bf9ce76fdf591c8',
    'launch_prepared.py': '2f3f7ea5a5260570eded799d55a0ad0bb065c3a90707ea1bd01b6fd1231c60e0',
}

def replace(text, before, after):
    assert text.count(before) == 1, 'adapter insertion changed: ' + before[:70]
    return text.replace(before, after)

def install(b):
    for name, digest in HELPERS.items():
        assert sha(TEMPLATE / name) == digest, 'proven runner changed: ' + name
        text = (TEMPLATE / name).read_text()
        text = text.replace('resident-api-small-r2', b.name.removeprefix('native-'))
        if name in ('run_validation.py', 'receipt_checks.py'):
            start = text.index('def manifest_dir(phase):')
            end = text.index('\n\ndef ', start + 4)
            text = text[:start] + ('def manifest_dir(phase):\n'
                '    return json.loads((HERE / ("contract-" + phase + "-r1.json")).read_text())["manifest_dir"]\n') + text[end:]
        if name == 'run_validation.py':
            text = re.sub(r'^LIMITS = .*$', 'LIMITS = ' + repr({
                p: spec[2] * MIB for p, spec in PHASES.items()}), text, flags=re.M)
            text = replace(text,
                'if path.name in {s["label"] + ".log" for s in contract["stages"]} else 1',
                'if path.name in {s["label"] + ".log" for s in contract["stages"]} or path.name in contract.get("allowed_empty_outputs", []) else 1')
            text = replace(text, '        memory = helper.memory',
                '        def protected_headroom():\n'
                '            values = {line.split(":")[0]: int(line.split()[1]) * 1024\n'
                '                      for line in Path("/proc/meminfo").read_text().splitlines()\n'
                '                      if line.startswith(("MemTotal:", "MemAvailable:"))}\n'
                '            return values["MemAvailable"] - max(1667398860, values["MemTotal"] // 10)\n'
                '        helper.headroom = protected_headroom\n'
                '        memory = helper.memory')
        if name == 'receipt_checks.py':
            text = re.sub(r'^PHASES = .*$', 'PHASES = ' + repr(tuple(PHASES)), text, flags=re.M)
            text = adapt_checks(text)
        if name == 'launch_prepared.py':
            text = adapt_launcher(text)
        ast.parse(text, filename=str(b / name))
        (b / name).write_text(text)
    (b / 'verify_phase.py').write_text(
        'from pathlib import Path\nimport json, sys\n'
        'sys.path.insert(0, str(Path(__file__).resolve().parent))\n'
        'from receipt_checks import validate\n'
        'result = validate(sys.argv[1])\n'
        'path = Path(__file__).resolve().parent / ("result-" + sys.argv[1] + ".json")\n'
        'with path.open("x") as stream: json.dump(result, stream, indent=2)\n'
        'print(json.dumps({"status": result["status"], "phase": result["phase"]}))\n')

def adapt_checks(text):
    text = replace(text,
        'if Path(name).name in {s["label"] + ".log" for s in contract["stages"]} else 1',
        'if Path(name).name in {s["label"] + ".log" for s in contract["stages"]} or Path(name).name in contract.get("allowed_empty_outputs", []) else 1')
    text = replace(text,
        'require(set(actual) == set(expected), "resolved source inventory changed")',
        'required = contract["required_dependency_sources"][name]\n'
        '            require(set(required) <= set(actual) <= set(expected), "resolved source inventory changed")')
    text = replace(text, 'result["source_dependencies"] = contract["expected_dependency_sources"]',
        'result["source_dependencies"] = {name: shlex.split((runtime / name).read_text().splitlines()[0].split(": ", 1)[1])\n'
        '                                         for name in contract["expected_dependency_sources"]}')
    text = replace(text, '    return result\n\n\ndef validate_inventory',
        '    result.update(contract_sha256=sha(HERE / ("contract-" + phase + "-r1.json")),\n'
        '                  manifest_sha256=sha(HERE / ("inputs-" + phase + "-r1.json")),\n'
        '                  source_snapshot_sha256=sha(HERE / "source-binding.json"))\n'
        '    return result\n\n\ndef validate_inventory')
    return text

def adapt_launcher(text):
    text = replace(text, 'phase = sys.argv[1]',
        'import fcntl\nlock = (HERE / "serial.lock").open("a")\n'
        'fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)\nphase = sys.argv[1]')
    text = replace(text, "contract = json.loads(contract_path.read_text())",
        "contract = json.loads(contract_path.read_text())\n"
        "assert contract['phase'] == phase\n"
        "for name, digest in contract['current_source_sha256'].items():\n"
        "    with Path(name).open('rb') as source:\n"
        "        assert hashlib.file_digest(source, 'sha256').hexdigest() == digest, 'current source changed: ' + name")
    return text
