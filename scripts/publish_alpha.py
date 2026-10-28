"""Create a draft from revalidated CI evidence and an explicit verified file list."""
import argparse
import json
from pathlib import Path
import subprocess
from check_release_run import REPO, snapshot
from prepare_alpha import prepare


def publish(directory, evidence_path):
    recorded = json.loads(evidence_path.read_text())
    actual, uploads = prepare(directory, recorded['ci'])
    if actual != recorded:
        raise ValueError('release evidence changed')
    if snapshot(recorded['ci']['run']['id']) != recorded['ci']:
        raise ValueError('CI attempt or artifact evidence changed')
    tag = recorded['packages'][0]['tag']
    commit = recorded['ci']['run']['head_sha']
    subprocess.run(['gh', 'api', '--method', 'POST', f'repos/{REPO}/git/refs',
                    '-f', 'ref=refs/tags/' + tag, '-f', 'sha=' + commit], check=True, timeout=30)
    subprocess.run(['gh', 'release', 'create', tag, '--repo', REPO, '--verify-tag',
                    '--draft', '--prerelease', '--latest=false', '--title', tag,
                    '--notes-file', 'release-notes.md',
                    *map(str, uploads), str(evidence_path)], check=True, timeout=300)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('evidence', type=Path)
    args = parser.parse_args()
    publish(args.directory, args.evidence)
