"""Prepare an exact upload list from packages bound to a successful CI attempt."""
import argparse
import json
import os
from pathlib import Path
from alpha_artifacts import inspect
from check_release_run import APPS, bind_artifacts
from download_alpha import download


def prepare(directory, evidence):
    bind_artifacts(evidence['run'], evidence['jobs'], evidence['artifacts'])
    commit = evidence['run']['head_sha']
    records, uploads = [], []
    for app in APPS:
        path = directory / app / f'{app}-linux-x86_64-{commit}.tar.gz'
        records.append(inspect(path, app, commit))
        uploads.extend([path, path.with_suffix('.gz.sha256')])
    actual = {p for p in directory.rglob('*') if p.name.endswith(('.tar.gz', '.sha256'))}
    if actual != set(uploads) or any(p.is_symlink() for p in directory.rglob('*')):
        raise ValueError('unexpected release upload files')
    if len({r['tag'] for r in records}) != 1:
        raise ValueError('application versions differ')
    return {'ci': evidence, 'packages': records}, uploads


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('evidence', type=Path)
    args = parser.parse_args()
    evidence = json.loads(args.evidence.read_text())
    bind_artifacts(evidence['run'], evidence['jobs'], evidence['artifacts'])
    download(evidence, args.directory)
    record, _ = prepare(args.directory, evidence)
    Path('release-evidence.json').write_text(json.dumps(record, indent=2) + '\n')
    Path('release-notes.md').write_text(f"Alpha from `{evidence['run']['head_sha']}`.\n\n"
        f"CI run {evidence['run']['id']}, attempt {evidence['run']['run_attempt']}.\n\n"
        'Browser and experimental native packages. No live native acceptance or service activation.\n')
    with open(os.environ['GITHUB_OUTPUT'], 'a') as output:
        output.write('tag=' + record['packages'][0]['tag'] + '\n')
