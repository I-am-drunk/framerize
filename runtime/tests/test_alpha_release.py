"""Release preparation runs on disposable files, never on service state."""
import copy
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
from alpha_artifacts import inspect
from check_release_run import validate, bind_artifacts, snapshot
from prepare_alpha import prepare
from publish_alpha import publish
from download_alpha import download

COMMIT = 'a' * 40


def fixture(directory, app, **changes):
    prefix = f'{app}-linux-x86_64-{COMMIT}'
    info = dict(commit=COMMIT, application=app, source_dirty=False,
                development=False, build_profile='release', platform='linux-x86_64',
                version='0.6.0-alpha.1', release_tag='v0.6.0-alpha.1',
                version_output=f'{app} 0.6.0-alpha.1 ({COMMIT})')
    info.update(changes)
    files = {'bin/' + app: b'never executed', 'runtime/runtime-manifest.json': b'{}',
             'build-info.json': json.dumps(info).encode()}
    files['manifest.json'] = json.dumps({name: hashlib.sha256(data).hexdigest()
                                      for name, data in files.items()}).encode()
    path = directory / (prefix + '.tar.gz')
    with tarfile.open(path, 'w:gz') as archive:
        for name, data in files.items():
            member = tarfile.TarInfo(prefix + '/' + name)
            member.size = len(data)
            archive.addfile(member, io.BytesIO(data))
    path.with_suffix('.gz.sha256').write_text(hashlib.sha256(path.read_bytes()).hexdigest() + '  ' + path.name + '\n')
    return path


class AlphaReleaseTests(unittest.TestCase):
    def test_stale_dirty_development_and_stable_packages_are_refused(self):
        for change in [dict(commit='b' * 40), dict(source_dirty=True),
                       dict(development=True), dict(build_profile='debug'),
                       dict(version='0.6.0'), dict(version_output='old binary')]:
            with self.subTest(change=change), tempfile.TemporaryDirectory() as directory:
                path = fixture(Path(directory), 'framerize', **change)
                with self.assertRaises(ValueError):
                    inspect(path, 'framerize', COMMIT)

    def test_changed_archive_checksum_is_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            path = fixture(Path(directory), 'framerize')
            with path.open('ab') as output:
                output.write(b'changed')
            with self.assertRaisesRegex(ValueError, 'checksum'):
                inspect(path, 'framerize', COMMIT)

    def test_ci_requires_completed_non_pr_run_and_every_named_job(self):
        run, jobs, _ = ci_fixture()
        self.assertEqual(validate(run, jobs), COMMIT)
        for change in [dict(event='pull_request'), dict(conclusion='failure'),
                       dict(status='in_progress'), dict(head_branch='topic'),
                       dict(repository={'full_name': 'other/framerize'})]:
            with self.subTest(change=change), self.assertRaises(ValueError):
                validate({**run, **change}, jobs)
        for job in jobs:
            for outcome in ['failure', 'skipped', 'cancelled', None]:
                changed = copy.deepcopy(jobs)
                next(j for j in changed if j['name'] == job['name'])['conclusion'] = outcome
                with self.subTest(job=job['name'], outcome=outcome), self.assertRaises(ValueError):
                    validate(run, changed)
        with self.assertRaises(ValueError):
            validate(run, jobs[:-1])


def ci_fixture():
    run = dict(id=123, run_attempt=1, repository={'full_name': 'I-am-drunk/framerize'},
               path='.github/workflows/ci.yml', event='push', head_branch='browserless',
               status='completed', conclusion='success', head_sha=COMMIT,
               html_url='https://github.com/I-am-drunk/framerize/actions/runs/123')
    jobs = [dict(id=i + 1, name=name, conclusion='success', status='completed',
                 run_id=123, run_attempt=1, head_sha=COMMIT,
                 started_at='2026-10-27T12:00:00Z', completed_at='2026-10-27T12:02:00Z')
            for i, name in enumerate(['build (framerize)', 'build (framerize-native)', 'Check'])]
    artifacts = [dict(id=i + 100, name=f'{app}-linux-x86_64-{COMMIT}-run-123-attempt-1',
                      digest='sha256:' + 'b' * 64, expired=False,
                      workflow_run={'id': 123, 'head_sha': COMMIT},
                      created_at='2026-10-27T12:01:00Z')
                 for i, app in enumerate(['framerize', 'framerize-native'])]
    return run, jobs, artifacts


def inputs(directory):
    run, jobs, artifacts = ci_fixture()
    evidence = dict(run=run, jobs=jobs, artifacts=bind_artifacts(run, jobs, artifacts))
    for app in ['framerize', 'framerize-native']:
        (directory / app).mkdir()
        fixture(directory / app, app)
    return evidence


class BoundReleaseTests(unittest.TestCase):
    def test_rejects_mixed_attempts_duplicate_names_and_unbound_artifacts(self):
        run, jobs, artifacts = ci_fixture()
        self.assertEqual(len(bind_artifacts(run, jobs, artifacts)), 2)
        for changed in [dict(run_attempt=2), dict(run_id=999), dict(head_sha='b' * 40)]:
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                validate(run, [{**jobs[0], **changed}, *jobs[1:]])
        with self.assertRaises(ValueError):
            validate(run, jobs + [jobs[0]])
        with self.assertRaises(ValueError):
            bind_artifacts(run, jobs, artifacts + [artifacts[0]])
        for changed in [dict(name='old-attempt'), dict(expired=True), dict(digest=None),
                        dict(workflow_run={'id': 999, 'head_sha': COMMIT}),
                        dict(created_at='2026-10-27T11:59:59Z')]:
            with self.subTest(changed=changed), self.assertRaises(ValueError):
                bind_artifacts(run, jobs, [{**artifacts[0], **changed}, artifacts[1]])

    def test_publisher_uploads_only_verified_files_and_retains_provenance(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            evidence = inputs(root)
            record, files = prepare(root, evidence)
            receipt = root / 'release-evidence.json'
            receipt.write_text(json.dumps(record))
            with patch('publish_alpha.snapshot', return_value=evidence), patch('publish_alpha.subprocess.run') as run:
                publish(root, receipt)
            argv = run.call_args_list[1].args[0]
            self.assertEqual(argv[-5:], [*map(str, files), str(receipt)])
            self.assertEqual(record['ci']['run']['run_attempt'], 1)
            self.assertEqual({a['id'] for a in record['ci']['artifacts']}, {100, 101})
            extra = root / 'framerize/unverified.tar.gz'
            extra.write_bytes(b'not a package')
            extra.with_suffix('.gz.sha256').write_text('not a checksum')
            with patch('publish_alpha.subprocess.run') as run, self.assertRaisesRegex(ValueError, 'unexpected'):
                publish(root, receipt)
            run.assert_not_called()

    def test_publisher_refuses_changed_attempt_before_any_write(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            evidence = inputs(root)
            record, _ = prepare(root, evidence)
            receipt = root / 'release-evidence.json'
            receipt.write_text(json.dumps(record))
            changed = copy.deepcopy(evidence)
            changed['run']['run_attempt'] = 2
            with patch('publish_alpha.snapshot', return_value=changed), patch('publish_alpha.subprocess.run') as run:
                with self.assertRaisesRegex(ValueError, 'evidence changed'):
                    publish(root, receipt)
            run.assert_not_called()

    def test_direct_commands_bind_downloads_and_reject_incomplete_jobs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            packages = root / 'packages'
            packages.mkdir()
            evidence = inputs(packages)
            for artifact in evidence['artifacts']:
                zipped = root / f"{artifact['id']}.zip"
                with zipfile.ZipFile(zipped, 'w') as z:
                    for path in (packages / artifact['application']).iterdir():
                        z.write(path, path.name)
                artifact['digest'] = 'sha256:' + hashlib.sha256(zipped.read_bytes()).hexdigest()
            endpoint = 'repos/I-am-drunk/framerize/actions/runs/123'
            mapping = {endpoint: evidence['run'],
                       endpoint + '/attempts/1/jobs?per_page=100': dict(total_count=3, jobs=evidence['jobs']),
                       endpoint + '/artifacts?per_page=100': dict(total_count=2, artifacts=evidence['artifacts'])}
            api = root / 'api.json'
            api.write_text(json.dumps(mapping))
            gh = root / 'gh'
            gh.write_text('#!' + sys.executable + '\nimport json,pathlib,sys\n'
                'root=pathlib.Path(__file__).parent\nargs=sys.argv[1:]\n'
                'with (root/"calls.jsonl").open("a") as f: f.write(json.dumps(args)+"\\n")\n'
                'if args[0]=="api" and args[-1].endswith("/zip"):\n'
                ' sys.stdout.buffer.write((root/(args[-1].split("/")[-2]+".zip")).read_bytes())\n'
                'elif len(args)==2 and args[0]=="api":\n'
                ' print(json.dumps(json.loads((root/"api.json").read_text())[args[1]]))\n'
                'else: print("{}")\n')
            gh.chmod(0o700)
            output = root / 'github-output'
            env = {**os.environ, 'PATH': str(root) + os.pathsep + os.environ['PATH'],
                   'GITHUB_OUTPUT': str(output)}
            def command(script, *args):
                return subprocess.run([sys.executable, '-B', str(ROOT / 'scripts' / script), *args],
                    cwd=root, env=env, capture_output=True, text=True, timeout=10)
            result = command('check_release_run.py', '123')
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(output.read_text(), 'sha=' + COMMIT + '\n')
            output.unlink()
            result = command('prepare_alpha.py', 'downloads', 'ci-evidence.json')
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(output.read_text(), 'tag=v0.6.0-alpha.1\n')
            records = json.loads((root / 'release-evidence.json').read_text())
            self.assertEqual({r['application'] for r in records['packages']}, {'framerize', 'framerize-native'})
            result = command('publish_alpha.py', 'downloads', 'release-evidence.json')
            self.assertEqual(result.returncode, 0, result.stderr)
            calls = [json.loads(line) for line in (root / 'calls.jsonl').read_text().splitlines()]
            downloads = [args[-1] for args in calls if args[-1].endswith('/zip')]
            self.assertEqual(downloads, ['repos/I-am-drunk/framerize/actions/artifacts/100/zip',
                                         'repos/I-am-drunk/framerize/actions/artifacts/101/zip'])
            output.unlink()
            mapping[endpoint + '/attempts/1/jobs?per_page=100']['total_count'] = 4
            api.write_text(json.dumps(mapping))
            result = command('check_release_run.py', '123')
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(output.exists())

    def test_download_refuses_wrong_digest_and_extra_zip_entries(self):
        for extra in [False, True]:
            with self.subTest(extra=extra), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                package_root = root / 'packages'
                package_root.mkdir()
                evidence = inputs(package_root)
                zipped = root / 'source.zip'
                with zipfile.ZipFile(zipped, 'w') as z:
                    for path in (package_root / 'framerize').iterdir():
                        z.write(path, path.name)
                    if extra:
                        z.writestr('unverified.tar.gz', b'extra')
                if extra:
                    evidence['artifacts'][0]['digest'] = 'sha256:' + hashlib.sha256(zipped.read_bytes()).hexdigest()
                def fake_download(*args, stdout, **kwargs):
                    stdout.write(zipped.read_bytes())
                with patch('download_alpha.subprocess.run', side_effect=fake_download):
                    with self.assertRaisesRegex(ValueError, 'inventory' if extra else 'digest'):
                        download(evidence, root / 'download')

    def test_snapshot_rechecks_the_run_after_attempt_specific_jobs(self):
        run, jobs, artifacts = ci_fixture()
        responses = [run, dict(total_count=3, jobs=jobs),
                     dict(total_count=2, artifacts=artifacts), {**run, 'run_attempt': 2}]
        with patch('check_release_run.get', side_effect=responses) as get:
            with self.assertRaisesRegex(ValueError, 'changed during inspection'):
                snapshot(123)
        self.assertIn('/attempts/1/jobs?', get.call_args_list[1].args[0])
