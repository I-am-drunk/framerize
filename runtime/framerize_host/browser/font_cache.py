"""Release unused file-cache pages after the browser scans its font bundle."""
import os
from pathlib import Path


def release_font_cache(executable):
    if not hasattr(os, 'posix_fadvise') or not hasattr(os, 'POSIX_FADV_DONTNEED'):
        return dict(outcome='unsupported', advised_files=0)
    fonts = Path(executable).resolve().parent / 'fonts'
    if not fonts.is_dir() or fonts.is_symlink():
        return dict(outcome='unavailable', advised_files=0,
                    cause='the browser font directory is absent or is a symlink')
    advised = 0
    try:
        for directory, children, names in os.walk(fonts, followlinks=False):
            children[:] = [name for name in children
                           if not (Path(directory) / name).is_symlink()]
            for name in names:
                path = Path(directory) / name
                if path.is_symlink() or not path.is_file():
                    continue
                with path.open('rb') as stream:
                    os.posix_fadvise(stream.fileno(), 0, 0, os.POSIX_FADV_DONTNEED)
                advised += 1
    except OSError as error:
        return dict(outcome='unavailable', advised_files=advised,
                    cause=f'font file-cache advice failed: {type(error).__name__}: {error.strerror}')
    return dict(outcome='ok', advised_files=advised)
