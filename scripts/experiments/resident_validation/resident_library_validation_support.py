"""Focused host/API fixture copies for the current resident source graph."""
from pathlib import Path
import ast, re, shutil, sys
ROOT = Path(__file__).resolve().parent
SUPPORT = ROOT / 'native-resident-worker-preparation'
sys.path.insert(0, str(SUPPORT))
from common import *
from snapshot import validate_snapshot
from adapters import install
from commands import build, required_sources
from inventory import CASE

EXTRA = {'host-tests-compile': 768, 'host-tests-run': 256,
         'api-tests-compile': 768, 'api-tests-run': 256}
TARGETS = {
    'host': [('sidecar_resident_tests.rs', 'sidecar::resident::tests::', 28),
             ('sidecar_resident_bootstrap_tests.rs', 'sidecar::resident_bootstrap::tests::', 5)],
    'api': [('experimental_native/resident_control_tests.rs',
             'experimental_native::control_authorization::resident_control::tests::', 17)],
}
KEEP = {'host': {'sidecar_resident.rs', 'sidecar_resident_bootstrap.rs'},
        'api': {'experimental_native/resident_control.rs'}}
MODULE = re.compile(r'#\[cfg\(test\)\](?=\s*(?:#\[path\s*=\s*"[^"\n]+"\]\s*)?(?:pub(?:\([^)]*\))?\s+)?mod\s)')

def preparation_files():
    return [Path(__file__).resolve(), ROOT / 'prepare_resident_library_validation.py', *SUPPORT.glob('*.py')]

def fixture_text(kind, relative, text):
    if relative in {name for name, _, _ in TARGETS[kind]}:
        return text
    if relative not in KEEP[kind]:
        text = MODULE.sub('#[cfg(any())]', text)
    text = CASE.sub(lambda m: '#[cfg(any())]\n' + m.group(0), text)
    return text

def cases(kind, source):
    result = []
    for relative, prefix, count in TARGETS[kind]:
        names = CASE.findall((source / 'src' / relative).read_text())
        assert len(names) == len(set(names)) == count, 'review library inventory'
        result += [prefix + name for name in names]
    return result

def freeze(b, base):
    binding = validate_snapshot(base)
    b.mkdir(mode=0o700)
    shutil.copyfile(base / 'source-binding.json', b / 'source-binding.json')
    changes, inventories = {}, {}
    for kind in TARGETS:
        source = base / 'source/crates' / ('framer-' + kind)
        target = b / 'fixture/crates' / ('framer-' + kind)
        shutil.copytree(source, target)
        inventories[kind] = cases(kind, source)
        for path in target.rglob('*.rs'):
            relative = str(path.relative_to(target / 'src'))
            original = path.read_text()
            selected = fixture_text(kind, relative, original)
            if selected != original:
                path.write_text(selected)
                changes[kind + '/' + relative] = dict(before=sha(source / 'src' / relative), after=sha(path))
    shutil.copytree(base / 'source/scripts', b / 'fixture/scripts')
    write(b / 'library-binding.json', dict(base=str(base), source_sha256=sha(base / 'source-binding.json'),
        changes=changes, cases=inventories, preparer_sha256={str(p): sha(p) for p in preparation_files()}))
    saved = dict(PHASES)
    PHASES.clear()
    PHASES.update({p: (p.split('-')[0], 'run' if p.endswith('-run') else 'test', cap, ()) for p, cap in EXTRA.items()})
    install(b)
    PHASES.clear(); PHASES.update(saved)
    launcher = b / 'launch_prepared.py'
    text = launcher.read_text().replace('(HERE / "serial.lock")', '(Path(' + repr(str(base)) + ') / "serial.lock")')
    ast.parse(text); launcher.write_text(text)
    validate(b)

def validate(b):
    item = read(b / 'library-binding.json')
    assert item['preparer_sha256'] == {str(p): sha(p) for p in preparation_files()}, 'preparer drift'
    base = Path(item['base'])
    binding = validate_snapshot(base)
    assert item['source_sha256'] == sha(base / 'source-binding.json') == sha(b / 'source-binding.json')
    changes = {}
    for kind in TARGETS:
        source = base / 'source/crates' / ('framer-' + kind)
        target = b / 'fixture/crates' / ('framer-' + kind)
        expected = {str(p.relative_to(source)) for p in source.rglob('*') if p.is_file()}
        actual = {str(p.relative_to(target)) for p in target.rglob('*') if p.is_file()}
        assert actual == expected, 'fixture file inventory drift'
        assert cases(kind, source) == cases(kind, target) == item['cases'][kind]
        for name in sorted(expected):
            old, new = source / name, target / name
            relative = str(old.relative_to(source / 'src')) if old.suffix == '.rs' else name
            content = fixture_text(kind, relative, old.read_text()) if old.suffix == '.rs' else old.read_text()
            assert new.read_text() == content, 'fixture drift: ' + kind + '/' + name
            if old.read_bytes() != new.read_bytes():
                changes[kind + '/' + relative] = dict(before=sha(old), after=sha(new))
    assert changes == item['changes']
    original = base / 'source/scripts'
    target = b / 'fixture/scripts'
    assert {p.name: sha(p) for p in original.iterdir()} == {p.name: sha(p) for p in target.iterdir()}
    return base, binding, item

def compile_stage(b, phase, pins):
    base, binding, item = validate(b)
    kind = phase.split('-')[0]
    argv, _, _, source, tree = build(base, kind + '-compile', pins)
    original = str(base / 'source')
    tree = b / 'fixture'
    candidate = tree / 'crates' / ('framer-' + kind) / 'src/lib.rs'
    argv[argv.index(str(source))] = str(candidate)
    argv[argv.index('--crate-type=rlib')] = '--test'
    argv = ['-Cmetadata=' + b.name.replace('-', '_') + '_' + phase.replace('-', '_')
            if arg.startswith('-Cmetadata=') else arg for arg in argv if not arg.startswith('--emit=')]
    runtime = b / (phase + '-r1')
    argv += ['-Cstrip=symbols', '-Clink-arg=-Wl,--no-keep-memory',
             '--emit=link=' + str(runtime / 'tests') + ',dep-info=' + str(runtime / 'tests.d')]
    assert len(argv) <= 64, 'accepted runner argv bound'
    directory = tree / 'crates' / ('framer-' + kind)
    paths = list(directory.rglob('*.rs')) + [directory / 'Cargo.toml']
    if kind == 'host':
        paths += list((tree / 'scripts').glob('*.py'))
    pins.update({str(p): sha(p) for p in paths})
    required = [p.replace(original + '/', str(tree) + '/')
                for p in required_sources(base, kind + '-compile', source, base / 'source')]
    required += [str(directory / 'src' / name) for name, _, _ in TARGETS[kind]]
    if kind == 'api':
        required += [str(directory / 'src/experimental_native/control_authorization_fixture.rs')]
    return ([dict(label='compile', argv=argv, timeout_seconds=100)],
            ['tests', 'tests.d', 'compile.log'], {'tests.d': sorted(set(required))},
            {'tests.d': sorted(str(p) for p in paths if p.suffix in ('.rs', '.py'))}, str(directory))
