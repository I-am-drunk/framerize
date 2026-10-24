"""Source-only supplement to the frozen resident worker validation graph."""
from pathlib import Path
import ast, re, shutil, sys
ROOT = Path(__file__).resolve().parent
SUPPORT = ROOT / 'native-resident-worker-preparation'
sys.path.insert(0, str(SUPPORT))
from common import *
from snapshot import validate_snapshot
from adapters import install, replace
from commands import build, required_sources
from inventory import CASE

EXTRA = {'main-tests-compile': 768, 'main-tests-run': 256, 'main-link-compile': 768}

def preparation_files():
    return [Path(__file__).resolve(), ROOT / 'prepare_resident_main_validation.py', *SUPPORT.glob('*.py')]

def main_cases(directory):
    cases = []
    for name, prefix, count in [('resident_socket_tests.rs', 'resident_socket::tests::', 29),
                                 ('resident_startup_tests.rs', 'resident_startup_tests::', 3)]:
        names = CASE.findall((directory / 'src' / name).read_text())
        assert len(names) == len(set(names)) == count, 'review changed inventory'
        cases.extend(prefix + name for name in names)
    return cases

def fixture_text(name, text):
    # Only the socket and CLI test module declarations stay enabled.
    text = text.replace('#[cfg(test)]', '#[cfg(any())]')
    for expression in re.findall(r'#\[cfg\(([^\n]*\btest\b[^\n]*)\)\]', text):
        allowed = (name == 'main.rs' and expression == 'all(test, feature = "native-dispatch")'
                   or name == 'resident_socket.rs' and expression == 'all(test, target_os = "linux")')
        assert allowed, 'unreviewed conditional test boundary: ' + name
    return text

def freeze(b, base):
    binding = validate_snapshot(base)
    b.mkdir(mode=0o700)
    shutil.copyfile(base / 'source-binding.json', b / 'source-binding.json')
    source = base / 'source/crates/framerize'
    target = b / 'main-fixture/crates/framerize'
    shutil.copytree(source, target)
    changes = {}
    for path in target.rglob('*.rs'):
        before = path.read_text()
        after = fixture_text(path.name, before)
        if after != before:
            path.write_text(after)
            changes[str(path.relative_to(target))] = {'before': sha(source / path.relative_to(target)), 'after': sha(path)}
    write(b / 'main-binding.json', dict(base=str(base), source_sha256=sha(base / 'source-binding.json'),
        fixture_changes=changes, cases=main_cases(target), status='SOURCE_ONLY_UNCOMPILED_UNRUN',
        preparer_sha256={str(p): sha(p) for p in preparation_files()}))
    # Adapt fresh copies only. The frozen parent graph and its pins stay unchanged.
    PHASES.clear()
    PHASES.update({p: ('framerize', 'run' if p.endswith('-run') else 'test', cap, ()) for p, cap in EXTRA.items()})
    install(b)
    short_temporary_root(b)
    launcher = b / 'launch_prepared.py'
    text = launcher.read_text().replace('(HERE / "serial.lock")', '(Path(' + repr(str(base)) + ') / "serial.lock")')
    ast.parse(text)
    launcher.write_text(text)
    validate(b)

def short_temporary_root(b):
    path = b / 'run_validation.py'
    text = path.read_text()
    insertion = '    module = types.ModuleType("accepted_dispatch_fixture_runner")'
    changes = [('"--setenv", "TMPDIR", str(scratch),', '"--setenv", "TMPDIR", "/tmp",'),
               ('"--bind", str(runtime), str(runtime), "--chdir", str(runtime),',
                '"--bind", str(runtime), str(runtime), "--bind", str(scratch), "/tmp", "--chdir", str(runtime),')]
    code = ''.join('    assert source.count(' + repr(old) + ') == 1\n'
                   '    source = source.replace(' + repr(old) + ', ' + repr(new) + ')\n'
                   for old, new in changes)
    text = replace(text, insertion, code + insertion)
    ast.parse(text)
    path.write_text(text)
    path = b / 'receipt_checks.py'
    text = replace(path.read_text(), '"--setenv", "TMPDIR", str(runtime / "tmp"),',
                   '"--setenv", "TMPDIR", "/tmp",')
    text = replace(text, '"--bind", str(runtime), str(runtime), "--chdir", str(runtime)]',
                   '"--bind", str(runtime), str(runtime), "--bind", str(runtime / "tmp"), "/tmp", "--chdir", str(runtime)]')
    ast.parse(text)
    path.write_text(text)

def validate(b):
    item = read(b / 'main-binding.json')
    assert item['preparer_sha256'] == {str(p): sha(p) for p in preparation_files()}, 'preparer drift'
    base = Path(item['base'])
    binding = validate_snapshot(base)
    assert item['source_sha256'] == sha(base / 'source-binding.json') == sha(b / 'source-binding.json')
    source = base / 'source/crates/framerize'
    target = b / 'main-fixture/crates/framerize'
    assert item['cases'] == main_cases(target), 'case inventory drift'
    expected = {str(p.relative_to(source)) for p in source.rglob('*') if p.is_file()}
    actual = {str(p.relative_to(target)) for p in target.rglob('*') if p.is_file()}
    assert actual == expected, 'main fixture file inventory drift'
    changes = {}
    for name in sorted(expected):
        old, new = source / name, target / name
        content = fixture_text(old.name, old.read_text()) if old.suffix == '.rs' else old.read_text()
        assert new.read_text() == content, 'main fixture changed: ' + name
        if old.read_bytes() != new.read_bytes():
            changes[name] = {'before': sha(old), 'after': sha(new)}
    assert changes == item['fixture_changes'], 'fixture change inventory drift'
    return base, binding, item

def compile_stage(b, phase, pins):
    base, binding, item = validate(b)
    argv, _, _, source, tree = build(base, 'main-check-compile', pins)
    test = phase == 'main-tests-compile'
    output = 'tests' if test else 'framerize'
    runtime = b / (phase + '-r1')
    if test:
        argv[argv.index('--crate-type=bin')] = '--test'
        tree = b / 'main-fixture'
        candidate = tree / 'crates/framerize/src/main.rs'
        argv[argv.index(str(source))] = str(candidate)
        source = candidate
    argv = ['-Cmetadata=' + b.name.replace('-', '_') + '_' + phase.replace('-', '_')
            if arg.startswith('-Cmetadata=') else arg for arg in argv if not arg.startswith('--emit=')]
    argv += ['-Cstrip=symbols', '-Clink-arg=-Wl,--no-keep-memory',
             '--emit=link=' + str(runtime / output) + ',dep-info=' + str(runtime / (output + '.d'))]
    assert len(argv) <= 64, 'accepted runner argv bound'
    directory = tree / 'crates/framerize'
    paths = list(directory.rglob('*.rs')) + [directory / 'Cargo.toml']
    pins.update({str(p): sha(p) for p in paths})
    required = required_sources(base, 'main-check-compile', source, tree)
    if test:
        old = str(base / 'source/crates/framerize')
        required = [p.replace(old, str(directory)) for p in required]
        required += [str(directory / 'src' / name) for name in ('resident_socket_tests.rs', 'resident_startup_tests.rs')]
    return ([dict(label='compile', argv=argv, timeout_seconds=100)],
            [output, output + '.d', 'compile.log'],
            {output + '.d': sorted(set(required))},
            {output + '.d': sorted(str(p) for p in paths if p.suffix == '.rs')}, str(directory))
