"""Validate preview ZIP data before writing into a new private directory.

Adapted from T3 stage-preview-bundle.py at fbe5df2d, MIT license.
"""
import re
import stat
import sys
import zipfile
from pathlib import Path

MAX_ARCHIVE = 36 * 1024 * 1024
MAX_PACKAGE = 16 * 1024 * 1024
MAX_JSON = 2 * 1024 * 1024
NUMBER = r"(?:0|[1-9][0-9]*)"
PACKAGE = re.compile(
    rf"framerize(?:-remote)?-{NUMBER}\.{NUMBER}\.{NUMBER}"
    r"-pr\.[1-9][0-9]*\.[1-9][0-9]*\.g[0-9a-f]{12}\.tgz"
)


def stage_preview(archive: Path, destination: Path):
    if archive.stat().st_size > MAX_ARCHIVE:
        raise ValueError("Preview archive is too large")
    with zipfile.ZipFile(archive) as bundle:
        entries = bundle.infolist()
        if len(entries) != 4:
            raise ValueError("Preview requires two packages and two manifests")
        seen = set()
        for entry in entries:
            name = entry.filename
            manifest = name in ("preview.json", "producer.json")
            if entry.orig_filename != name or not (manifest or PACKAGE.fullmatch(name)):
                raise ValueError("Unexpected preview path")
            if entry.is_dir() or stat.S_IFMT(entry.external_attr >> 16) not in (0, stat.S_IFREG):
                raise ValueError("Preview inputs must be regular files")
            if name.casefold() in seen or entry.flag_bits & 1:
                raise ValueError("Duplicate or encrypted preview entry")
            seen.add(name.casefold())
            limit = MAX_JSON if manifest else MAX_PACKAGE
            if entry.file_size > limit or entry.file_size < 1:
                raise ValueError("Invalid preview file size")
        if not {"preview.json", "producer.json"} <= seen:
            raise ValueError("Preview manifests are missing")
        destination.mkdir(parents=True, exist_ok=False)
        for entry in entries:
            with bundle.open(entry) as source, (destination / entry.filename).open("xb") as output:
                remaining = entry.file_size
                while chunk := source.read(min(65536, remaining + 1)):
                    remaining -= len(chunk)
                    if remaining < 0:
                        raise ValueError("Preview file exceeds its declared size")
                    output.write(chunk)


if __name__ == "__main__":
    stage_preview(Path(sys.argv[1]), Path(sys.argv[2]))
