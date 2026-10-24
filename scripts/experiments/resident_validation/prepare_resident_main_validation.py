"""Prepare socket/CLI tests and linked main. Never launch a compiler or fixture."""
from pathlib import Path
import argparse, sys
sys.path.insert(0, str(Path(__file__).resolve().parent))
from resident_main_validation_support import *

def prepare(b, phase):
    base, binding, item = validate(b)
    pins = {}
    contract = read(TEMPLATE / 'contract-send-api-tests-compile-r1.json')
    assert not (b / (phase + '-r1')).exists(), 'attempt already consumed'
    if phase == 'main-tests-run':
        upstream(b, 'main-tests', pins)
        binary = b / 'main-tests-compile-r1/tests'
        cases = item['cases']
        stages = [dict(label='case-list', argv=[str(binary), '--list'], timeout_seconds=5),
                  dict(label='selected-list', argv=[str(binary), '--list', '--exact', *cases], timeout_seconds=5),
                  dict(label='tests', argv=[str(binary), '--test-threads=1', '--exact', *cases], timeout_seconds=30)]
        outputs = ['case-list.log', 'selected-list.log', 'tests.log']
        required, allowed, manifest_dir = {}, {}, str(b)
        contract.update(expected_case_names=cases, expected_all_case_names=cases, expected_filtered_count=0)
    else:
        stages, outputs, required, allowed, manifest_dir = compile_stage(b, phase, pins)
    contract.update(phase=phase, stages=stages, outputs=outputs, memory_max_bytes=EXTRA[phase] * MIB,
        deadline_seconds=110, outer_timeout_seconds=132, manifest_dir=manifest_dir,
        expected_dependency_sources=allowed, required_dependency_sources=required,
        future_inputs=[], prerequisites=[], allowed_empty_outputs=[],
        current_source_sha256={str(REPO / name): digest for name, digest in binding['files'].items()},
        scope='Focused current-main socket/CLI tests or linked executable; no activation.',
        source_acceptance='UNCOMPILED_UNRUN until matching bounded phase receipts pass.',
        dependency_pin_scope='Verified current-graph rlibs and pinned direct externs; sysroot/transitive cache trusted.',
        capture_policy='Exactly 28 socket and 3 CLI cases, one test thread; linking does not run main.')
    contract['argv_without_manifest_digest'] = [
        '--unit=framerize-browserless-' + b.name.removeprefix('native-') + '-' + phase + '-20261024-r1'
        if arg.startswith('--unit=') else '--property=MemoryMax=' + str(EXTRA[phase]) + 'M'
        if arg.startswith('--property=MemoryMax=') else str(b / 'run_validation.py')
        if arg == str(TEMPLATE / 'run_validation.py') else phase
        if arg == 'send-api-tests-compile' else arg for arg in contract['argv_without_manifest_digest']]
    support = [Path(__file__).resolve(), ROOT / 'resident_main_validation_support.py',
               b / 'main-binding.json', b / 'source-binding.json']
    support += list(SUPPORT.glob('*.py')) + list(b.glob('*.py'))
    support += [base / 'source-binding.json', ROOT.parent / 'browserless-resume-20261020/run_offline.py',
                ROOT.parent / 'ai-result-ownership-20261020/run_app_build.py',
                Path('/usr/bin/python3').resolve(), Path('/usr/bin/bwrap').resolve()]
    pins.update({str(p): sha(p) for p in support})
    validate(b)
    path = b / ('contract-' + phase + '-r1.json')
    contract.update(prepared_inputs=pins, required_inputs=sorted(set(pins) | {str(path)}))
    write(path, contract)
    write(b / ('inputs-' + phase + '-r1.json'), {'sha256': pins | {str(path): sha(path)}})
    print(json.dumps(dict(status='PREPARED_UNRUN', phase=phase, pins=len(pins), directory=str(b))))

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--directory', default='native-resident-main-validation-r1')
    parser.add_argument('--base', default='native-resident-worker-validation-r1')
    action = parser.add_mutually_exclusive_group(required=True)
    action.add_argument('--snapshot', action='store_true')
    action.add_argument('--phase', choices=EXTRA)
    args = parser.parse_args()
    assert re.fullmatch(r'native-resident-main-[a-z0-9-]+', args.directory)
    assert re.fullmatch(r'native-resident-worker-[a-z0-9-]+', args.base)
    b = ROOT / args.directory
    if args.snapshot:
        freeze(b, ROOT / args.base)
        print(json.dumps(dict(status='SOURCE_ONLY_UNCOMPILED_UNRUN', directory=str(b), phases=list(EXTRA))))
    else:
        prepare(b, args.phase)

if __name__ == '__main__':
    main()
