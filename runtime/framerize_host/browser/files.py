"""Copy immutable runtime inputs across filesystem and ownership boundaries."""
import errno
import os
import shutil


def link_or_copy(source, destination):
    try:
        os.link(source, destination)
    except OSError as error:
        if error.errno not in (errno.EXDEV, errno.EPERM, errno.EACCES, errno.EMLINK):
            raise
        shutil.copy2(source, destination)
    return destination
