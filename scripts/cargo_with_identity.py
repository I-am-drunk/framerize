"""Refresh source identity before Cargo evaluates its incremental build cache."""
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def environment(root):
    def git(*args):
        return subprocess.check_output(['git', '--no-optional-locks', '-C', str(root),
                                        *args], text=True).strip()
    commit = git('rev-parse', 'HEAD')
    dirty = bool(git('status', '--porcelain', '--untracked-files=normal'))
    return {**os.environ, 'FRAMERIZE_SOURCE_COMMIT': commit,
            'FRAMERIZE_SOURCE_DIRTY': str(dirty).lower()}


if __name__ == '__main__':
    os.execvpe('cargo', ['cargo', *sys.argv[1:]], environment(ROOT))
