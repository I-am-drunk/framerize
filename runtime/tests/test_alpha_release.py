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

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts'))
from alpha_artifacts import inspect
from check_release_run import validate

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
    def test_ci_command_entry_accepts_only_complete_job_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = dict(repository={'full_name': 'I-am-drunk/framerize'},
                       path='.github/workflows/ci.yml', event='push',
                       head_branch='browserless', status='completed',
                       conclusion='success', head_sha=COMMIT)
            jobs = [dict(name=name, conclusion='success') for name in
                    ['build (framerize)', 'build (framerize-native)', 'Check']]
            (root / 'run.json').write_text(json.dumps(run))
            job_file = root / 'jobs.json'
            job_file.write_text(json.dumps(dict(total_count=3, jobs=jobs)))
            gh = root / 'gh'
            gh.write_text('#!' + sys.executable + '\nimport pathlib,sys\n'
                          'name="jobs.json" if "/jobs?" in sys.argv[-1] else "run.json"\n'
                          'print(pathlib.Path(__file__).with_name(name).read_text())\n')
            gh.chmod(0o700)
            output = root / 'github-output'
            env = {**os.environ, 'PATH': str(root) + os.pathsep + os.environ['PATH'],
                   'GITHUB_OUTPUT': str(output)}
            command = [sys.executable, '-B', str(ROOT / 'scripts/check_release_run.py'), '123']
            result = subprocess.run(command, env=env, capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(output.read_text(), 'sha=' + COMMIT + '\n')
            output.unlink()
            job_file.write_text(json.dumps(dict(total_count=4, jobs=jobs)))
            result = subprocess.run(command, env=env, capture_output=True, timeout=10)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(output.exists())

    def test_direct_command_prepares_both_packages_without_executing_them(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for app in ['framerize', 'framerize-native']:
                fixture(root, app)
            output = root / 'github-output'
            result = subprocess.run([sys.executable, '-B', str(ROOT / 'scripts/prepare_alpha.py'),
                                     str(root), COMMIT], cwd=root,
                                    env={**os.environ, 'GITHUB_OUTPUT': str(output)},
                                    capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(output.read_text(), 'tag=v0.6.0-alpha.1\n')
            records = json.loads((root / 'release-evidence.json').read_text())
            self.assertEqual({r['application'] for r in records}, {'framerize', 'framerize-native'})

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
        run = dict(repository={'full_name': 'I-am-drunk/framerize'},
                   path='.github/workflows/ci.yml', event='push',
                   head_branch='browserless', status='completed',
                   conclusion='success', head_sha=COMMIT)
        jobs = [dict(name=name, conclusion='success') for name in
                ['build (framerize)', 'build (framerize-native)', 'Check']]
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
