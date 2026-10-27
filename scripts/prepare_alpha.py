"""Produce an alpha release draft's inputs from two already-tested CI packages."""
import argparse
import json
import os
from pathlib import Path
import re
from alpha_artifacts import inspect


def prepare(directory, commit):
    if not re.fullmatch('[0-9a-f]{40}', commit):
        raise ValueError('a full source commit is required')
    records = []
    for app in ['framerize', 'framerize-native']:
        matches = list(directory.rglob(f'{app}-linux-x86_64-{commit}.tar.gz'))
        if len(matches) != 1:
            raise ValueError('exactly one package per application is required')
        records.append(inspect(matches[0], app, commit))
    if len({r['tag'] for r in records}) != 1:
        raise ValueError('application versions differ')
    return records


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('commit')
    args = parser.parse_args()
    records = prepare(args.directory, args.commit)
    Path('release-evidence.json').write_text(json.dumps(records, indent=2) + '\n')
    Path('release-notes.md').write_text(
        f"Alpha built from `{args.commit}`. Packages reuse the successful CI artifacts.\n\n"
        "Browser serving and experimental native serving remain separate. "
        "This release does not establish live native acceptance or activate a service.\n")
    with open(os.environ['GITHUB_OUTPUT'], 'a') as output:
        output.write('tag=' + records[0]['tag'] + '\n')
