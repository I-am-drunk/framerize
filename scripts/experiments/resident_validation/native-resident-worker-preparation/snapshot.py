"""Freeze current public source; never copy private state or target trees."""
import re, shutil
from common import *
from inventory import RESIDENT, MAIN_REQUIRED, CASE, resident_cases, require_main_modules

SERVER_CASES = ['resident_control_refuses_missing_or_busy_slot_without_browser_io',
                'resident_control_retains_management_and_cleanup_fences']

def snapshot(b):
    b.mkdir(mode=0o700)
    files = {}
    for crate in ('framer-core', 'framer-host', 'framer-api', 'framer-server',
                  'framer-store', 'framerize'):
        paths = sorted((REPO / 'crates' / crate / 'src').rglob('*.rs'))
        paths.append(REPO / 'crates' / crate / 'Cargo.toml')
        for path in paths:
            relative = str(path.relative_to(REPO))
            files[relative] = sha(path)
    for name in ('camoufox_host', 'resident_activity', 'resident_authorization',
                 'contextual_authorization', 'resident_bootstrap'):
        relative = 'scripts/' + name + '.py'
        files[relative] = sha(REPO / relative)
    for relative, digest in files.items():
        target = b / 'source' / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(REPO / relative, target)
        assert sha(target) == digest
    cli = b / 'source/crates/framerize/src'
    require_main_modules(cli)
    cases = resident_cases(cli)
    (b / 'resident_fixture.rs').write_text(
        '#[cfg(feature = "native-dispatch")]\n#[path = "source/crates/framerize/src/'
        'resident_control.rs"]\nmod resident_control;\n')
    changes = server_fixture(b)
    pool = ROOT / 'native-resident-pool-r1'
    proof = read(pool / 'result.json')
    assert proof['passed'] is True and len(proof['cases']) == 16
    reused = {}
    for name in ('ids.rs', 'resident_pool.rs', 'resident_pool_tests.rs'):
        old = pool / 'source' / name
        current = b / 'source/crates/framer-core/src' / name
        assert proof['inputs'][str(old)] == sha(old) == sha(current), 'pool16 source drift'
        reused[str(old)] = sha(old)
    write(b / 'pool-reuse.json', dict(proof=str(pool / 'result.json'),
        proof_sha256=sha(pool / 'result.json'), source_sha256=reused,
        cases=proof['cases'], limit='Same pool/ids sources only; no fresh execution.'))
    for relative, digest in files.items():
        assert sha(REPO / relative) == digest, 'source changed while freezing: ' + relative
    write(b / 'source-binding.json', dict(status='SOURCE_ONLY_UNCOMPILED_UNRUN',
        repository=str(REPO), files=files, server_fixture_changes=changes,
        resident_fixture_sha256=sha(b / 'resident_fixture.rs'),
        resident_cases=cases, resident_case_counts=dict(coordinator=15, bootstrap_adapter=2),
        main_required_modules=list(MAIN_REQUIRED),
        server_cases=['pool_control::tests::native_control_tests::' + n for n in SERVER_CASES],
        limits='Inert seams only. No installed owner, live handoff or memory claim.'))

def server_fixture(b):
    source = b / 'source/crates/framer-server/src'
    target = b / 'server-fixture/crates/framer-server/src'
    shutil.copytree(source, target)
    changes = {}
    for path in target.rglob('*.rs'):
        text = original = path.read_text()
        if path.name != 'pool_control.rs':
            text = text.replace('#[cfg(test)]', '#[cfg(any())]')
        if path.name in ('pool_control_tests.rs', 'native_control_tests.rs'):
            keep = SERVER_CASES if path.name == 'native_control_tests.rs' else []
            text = CASE.sub(lambda m: m.group(0) if m.group(1) in keep else
                            '#[cfg(any())]\n' + m.group(0), text)
            disabled = ('native_merge_tests', 'ownership_merge_tests') if not keep else ('updates', 'routing')
            for name in disabled:
                needle = 'mod ' + name + ';'
                assert text.count(needle) == 1, 'fixture module boundary changed'
                text = text.replace(needle, '#[cfg(any())]\n' + needle)
        if text != original:
            path.write_text(text)
            changes[str(path.relative_to(target))] = dict(
                before_sha256=sha(source / path.relative_to(target)), after_sha256=sha(path))
    assert set(SERVER_CASES) <= set(CASE.findall((source / 'native_control_tests.rs').read_text()))
    return changes

def validate_snapshot(b):
    binding = read(b / 'source-binding.json')
    assert sha(b / 'resident_fixture.rs') == binding['resident_fixture_sha256']
    cli = b / 'source/crates/framerize/src'
    require_main_modules(cli)
    assert binding['resident_cases'] == resident_cases(cli), 'resident inventory changed'
    assert binding['resident_case_counts'] == dict(coordinator=15, bootstrap_adapter=2)
    assert binding['main_required_modules'] == list(MAIN_REQUIRED)
    for relative, digest in binding['files'].items():
        assert sha(b / 'source' / relative) == digest, 'frozen source drift: ' + relative
        assert sha(REPO / relative) == digest, 'current source drift: ' + relative
    prefix = 'crates/framer-server/src/'
    for path in (b / 'server-fixture/crates/framer-server/src').rglob('*.rs'):
        relative = str(path.relative_to(b / 'server-fixture/crates/framer-server/src'))
        change = binding['server_fixture_changes'].get(relative)
        expected = change['after_sha256'] if change else binding['files'][prefix + relative]
        assert sha(path) == expected, 'server fixture drift: ' + relative
    return binding
