"""Download only bound artifact IDs, checking GitHub ZIP digests before unpacking."""
import hashlib
from pathlib import Path
import shutil
import subprocess
import zipfile
from check_release_run import REPO


def download(evidence, directory):
    directory.mkdir()
    for artifact in evidence['artifacts']:
        app = artifact['application']
        root = directory / app
        root.mkdir()
        zipped = directory / f"{artifact['id']}.zip"
        with zipped.open('xb') as output:
            subprocess.run(['gh', 'api', f"repos/{REPO}/actions/artifacts/{artifact['id']}/zip"],
                           stdout=output, check=True, timeout=300)
        with zipped.open('rb') as source:
            digest = 'sha256:' + hashlib.file_digest(source, 'sha256').hexdigest()
        if digest != artifact['digest']:
            raise ValueError('downloaded artifact digest differs')
        prefix = f"{app}-linux-x86_64-{evidence['run']['head_sha']}.tar.gz"
        with zipfile.ZipFile(zipped) as archive:
            members = archive.infolist()
            if (len(members) != 2 or {m.filename for m in members} != {prefix, prefix + '.sha256'}
                    or sum(m.file_size for m in members) > 1024 ** 3):
                raise ValueError('unexpected artifact ZIP inventory')
            for member in members:
                with archive.open(member) as source, (root / member.filename).open('xb') as output:
                    shutil.copyfileobj(source, output)
