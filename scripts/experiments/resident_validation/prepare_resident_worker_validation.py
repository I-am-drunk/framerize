"""ROOT-ONLY preparation. No compiler, test, subprocess, network or service call."""
from pathlib import Path
import argparse, re, sys
SUPPORT = Path(__file__).resolve().parent / 'native-resident-worker-preparation'
sys.path.insert(0, str(SUPPORT))
from common import *
from snapshot import snapshot, validate_snapshot
from adapters import install
from commands import build, required_sources
from inventory import RESIDENT

def prepare(b, phase):
    binding = validate_snapshot(b)
    crate, mode, cap, deps = PHASES[phase]
    pins = {}
    contract = read(TEMPLATE / 'contract-send-api-tests-compile-r1.json')
    runtime = b / (phase + '-r1')
    assert not runtime.exists(), 'attempt already consumed'
    if mode == 'run':
        stages, outputs, required, allowed, manifest_dir = run_phase(b, phase, pins, binding)
    else:
        argv, output, depfile, source, tree = build(b, phase, pins)
        stages = [dict(label='compile', argv=argv, timeout_seconds=100)]
        outputs = [output, depfile, 'compile.log']
        required = {depfile: required_sources(b, phase, source, tree)}
        directory = tree / 'crates' / crate
        paths = list(directory.rglob('*.rs')) + [directory / 'Cargo.toml']
        if phase.startswith('resident-'):
            paths = [b / 'resident_fixture.rs', b / 'source/crates/framerize/src/main.rs']
            paths += [b / 'source/crates/framerize/src' / name for name in RESIDENT]
        if crate == 'framer-host':
            paths += list((b / 'source/scripts').glob('*.py'))
        if not (directory / 'Cargo.toml').exists():
            paths = [p for p in paths if p.name != 'Cargo.toml']
        pins.update({str(p): sha(p) for p in paths})
        allowed = {depfile: sorted(str(p) for p in paths if p.suffix in ('.rs', '.py'))}
        manifest_dir = str(directory)
    contract.update(phase=phase, stages=stages, outputs=outputs,
        memory_max_bytes=cap * MIB, deadline_seconds=110, outer_timeout_seconds=132,
        manifest_dir=manifest_dir, expected_dependency_sources=allowed,
        required_dependency_sources=required, future_inputs=[], prerequisites=[],
        allowed_empty_outputs=['checked.rmeta'] if mode == 'metadata' else [],
        current_source_sha256={str(REPO / p): digest for p, digest in binding['files'].items()},
        scope='Current resident worker source only. No activation or live acceptance.',
        source_acceptance='UNCOMPILED_UNRUN until matching bounded phase receipts pass.',
        dependency_pin_scope='Fresh project rlibs; pinned direct externs and accepted recipe receipts. Transitive cache and sysroot trusted.',
        capture_policy='Exact 15 coordinator + 2 bootstrap adapter cases, or two existing-admission cases; one test thread.')
    if mode == 'run':
        cases = binding['resident_cases' if phase == 'resident-run' else 'server_cases']
        contract.update(expected_case_names=cases, expected_all_case_names=cases,
                        expected_filtered_count=0)
    launch = contract['argv_without_manifest_digest']
    contract['argv_without_manifest_digest'] = [
        '--unit=framerize-browserless-' + b.name.removeprefix('native-') + '-' + phase + '-20261024-r1'
        if arg.startswith('--unit=') else '--property=MemoryMax=' + str(cap) + 'M'
        if arg.startswith('--property=MemoryMax=') else str(b / 'run_validation.py')
        if arg == str(TEMPLATE / 'run_validation.py') else phase
        if arg == 'send-api-tests-compile' else arg for arg in launch]
    support = [Path(__file__).resolve(), b / 'source-binding.json', b / 'pool-reuse.json']
    support += list(SUPPORT.glob('*.py')) + list(b.glob('*.py'))
    support += [ROOT.parent / 'browserless-resume-20261020/run_offline.py',
        ROOT.parent / 'ai-result-ownership-20261020/run_app_build.py',
        Path('/usr/bin/python3').resolve(), Path('/usr/bin/bwrap').resolve()]
    pins.update({str(p): sha(p) for p in support})
    validate_snapshot(b)
    path = b / ('contract-' + phase + '-r1.json')
    contract.update(prepared_inputs=pins, required_inputs=sorted(set(pins) | {str(path)}))
    write(path, contract)
    write(b / ('inputs-' + phase + '-r1.json'), {'sha256': pins | {str(path): sha(path)}})
    print(json.dumps(dict(status='PREPARED_UNRUN', phase=phase, pins=len(pins), directory=str(b))))

def run_phase(b, phase, pins, binding):
    kind = phase.removesuffix('-run')
    upstream(b, kind + '-tests', pins)
    binary = b / (kind + '-tests-compile-r1') / 'tests'
    cases = binding[kind + '_cases']
    stages = [dict(label='case-list', argv=[str(binary), '--list'], timeout_seconds=5),
        dict(label='selected-list', argv=[str(binary), '--list', '--exact', *cases], timeout_seconds=5),
        dict(label='tests', argv=[str(binary), '--test-threads=1', '--exact', *cases], timeout_seconds=30)]
    return stages, ['case-list.log', 'selected-list.log', 'tests.log'], {}, {}, str(b)

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--directory', required=True)
    action = parser.add_mutually_exclusive_group(required=True)
    action.add_argument('--snapshot', action='store_true')
    action.add_argument('--phase', choices=PHASES)
    args = parser.parse_args()
    assert re.fullmatch(r'native-resident-worker-[a-z0-9-]+', args.directory)
    assert args.directory != SUPPORT.name
    b = ROOT / args.directory
    if args.snapshot:
        snapshot(b)
        install(b)
        print(json.dumps(dict(status='SOURCE_ONLY_UNCOMPILED_UNRUN', directory=str(b),
                              next_phase='core-compile', phases=list(PHASES))))
    else:
        prepare(b, args.phase)

if __name__ == '__main__':
    main()
