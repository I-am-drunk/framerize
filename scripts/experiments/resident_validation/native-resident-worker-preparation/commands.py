"""Create commands from saved successful recipes, replacing every project rlib."""
from common import *
from inventory import RESIDENT, MAIN_REQUIRED

def saved_recipe(phase, pins):
    directory, old_phase = recipe(phase)
    base = ROOT / directory
    contract = base / ('contract-' + old_phase + '-r1.json')
    proof = base / ('result-' + old_phase + '.json')
    manifest = base / ('inputs-' + old_phase + '-r1.json')
    assert read(proof)['status'] == 'pass', 'missing proven recipe: ' + str(proof)
    pins.update({str(p): sha(p) for p in (contract, proof, manifest)})
    # Trust the saved acceptance receipt, not an unbounded recursive history audit.
    return read(contract)['stages'][0]['argv'], read(manifest)['sha256']

def build(b, phase, pins):
    crate, mode, cap, deps = PHASES[phase]
    original, old_pins = saved_recipe(phase, pins)
    for dep in deps:
        upstream(b, dep, pins)
    cli = crate == 'framerize'
    small = phase.startswith('resident-')
    fixture = phase == 'server-tests-compile'
    tree = b / ('server-fixture' if fixture else 'source')
    source = b / 'resident_fixture.rs' if small else tree / 'crates' / crate / 'src' / ('main.rs' if cli else 'lib.rs')
    runtime = b / (phase + '-r1')
    output = 'tests' if mode == 'test' else 'checked.rmeta' if mode == 'metadata' else 'lib' + crate.replace('-', '_') + '.rlib'
    depfile = output + '.d'
    argv = [original[0], '--edition=2021', '--crate-name', crate.replace('-', '_'),
            '--test' if mode == 'test' else '--crate-type=' + ('bin' if cli and not small else 'rlib'),
            str(source), '-Cdebuginfo=0', '-Copt-level=0', '-Ccodegen-units=128',
            '-Cmetadata=' + b.name.replace('-', '_') + '_' + phase.replace('-', '_'),
            '--cfg', 'feature="native-dispatch"', '--cfg', 'feature="experimental-native"']
    if mode == 'test':
        argv += ['-Cstrip=symbols', '-Clink-arg=-Wl,--no-keep-memory']
    for dep in deps:
        argv += ['-L', 'dependency=' + str(artifact(b, dep).parent)]
    argv += ['-L', 'dependency=' + str(CACHE)]
    if phase in ('store-compile', 'main-check-compile'):
        for i, value in enumerate(original[:-1]):
            if value == '-L' and (original[i + 1].startswith('native=') or
                'worker-jitless-owner-build-r1/runs/compile-1' in original[i + 1] or
                'mint-sha2-compile-r1' in original[i + 1]):
                argv += ['-L', original[i + 1]]
                if original[i + 1].startswith('native='):
                    for path in Path(original[i + 1].split('=', 1)[1]).glob('*.a'):
                        assert old_pins[str(path)] == sha(path), 'native archive changed'
                        pins[str(path)] = old_pins[str(path)]
    small_names = {'framer_core', 'framer_host', 'framer_api', 'framer_server',
                   'futures', 'tokio', 'serde_json', 'uuid'}
    for arg in original:
        if not arg.startswith('--extern='):
            continue
        _, name, path = arg.split('=', 2)
        if small and name not in small_names:
            continue
        if name.startswith('framer_'):
            dependency = name.removeprefix('framer_')
            assert dependency in deps, 'unplanned production crate dependency'
            path = str(artifact(b, dependency))
            assert Path(path).suffix == '.rlib' and path in pins
        else:
            assert old_pins[path] == sha(path), 'direct dependency changed: ' + path
            pins[path] = old_pins[path]
        argv.append('--extern=' + name + '=' + path)
    if phase == 'resident-tests-compile':
        test_recipe, test_pins = saved_recipe('host-compile', pins)
        macros = [arg for arg in test_recipe if arg.startswith('--extern=async_trait=')]
        assert len(macros) == 1, 'missing pinned test macro'
        macro = macros[0].split('=', 2)[2]
        assert test_pins[macro] == sha(macro), 'test macro changed'
        pins[macro] = test_pins[macro]
        argv += macros
    assert old_pins[argv[0]] == sha(argv[0]), 'compiler changed'
    pins[argv[0]] = old_pins[argv[0]]
    if phase == 'main-check-compile':
        worker_reuse(pins, old_pins)
    emit = 'metadata' if mode == 'metadata' else 'link'
    argv += ['--emit=' + emit + '=' + str(runtime / output) + ',dep-info=' + str(runtime / depfile)]
    assert len(argv) <= 64, 'accepted runner argv bound'
    return argv, output, depfile, source, tree

def worker_reuse(pins, old_pins):
    previous = ROOT / 'worker-jitless-owner-build-r1/source'
    paths = sorted(previous.glob('*.rs'))
    assert len(paths) == 5, 'worker source inventory changed'
    for path in paths:
        current = REPO / 'crates/offline-worker-owner' / path.name
        assert sha(path) == sha(current) == old_pins[str(path)], 'worker source changed'
        pins[str(path)] = sha(path)
        pins[str(current)] = sha(current)

def required_sources(b, phase, source, tree):
    crate = PHASES[phase][0]
    base = tree / 'crates' / crate / 'src'
    required = [source]
    if phase.startswith('resident-') or phase == 'main-check-compile':
        required += [b / 'source/crates/framerize/src' / n for n in RESIDENT
                     if phase == 'resident-tests-compile' or not n.endswith('_tests.rs')]
        if phase == 'main-check-compile':
            required += [base / n for n in MAIN_REQUIRED]
    elif crate == 'framer-core':
        required += [base / 'resident_pool.rs']
    elif crate == 'framer-host':
        required += [base / n for n in ('sidecar.rs', 'sidecar_resident.rs',
                                       'sidecar_resident_bootstrap.rs')]
        required += list((b / 'source/scripts').glob('*.py'))
    elif crate == 'framer-api':
        required += [base / 'experimental_native' / n for n in
                     ('control_authorization.rs', 'resident_control.rs',
                      'session_store.rs', 'session_versions.rs', 'session_account_change.rs')]
    elif crate == 'framer-server':
        required += [base / n for n in ('native_control.rs', 'pool_control.rs')]
        if phase == 'server-tests-compile':
            required += [base / n for n in ('native_control_tests.rs', 'pool_control_tests.rs')]
    return sorted(set(map(str, required)))
