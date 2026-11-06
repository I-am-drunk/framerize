"""Copy immutable runtime inputs across filesystem and ownership boundaries."""
import errno
import os
from pathlib import Path
import shutil
import subprocess


def link_or_copy(source, destination):
    try:
        os.link(source, destination)
    except OSError as error:
        if error.errno not in (errno.EXDEV, errno.EPERM, errno.EACCES, errno.EMLINK):
            raise
        shutil.copy2(source, destination)
    return destination


def normalize_text_line_endings(data):
    return data.replace(b"\r\n", b"\n")


def apply_unified_patch(target, patch_file):
    """Apply a unified diff consistently when checkouts change its line endings."""
    data = normalize_text_line_endings(Path(patch_file).read_bytes())
    subprocess.run(
        ["patch", "--batch", "--fuzz=0", str(target), "-"],
        input=data,
        check=True,
        capture_output=True,
        timeout=30,
    )
