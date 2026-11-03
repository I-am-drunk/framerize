"""Check both package readers before changing a service with signed history."""
from contextlib import closing
import sqlite3


def formats(info, *, allow_legacy=False):
    fallback = dict(read_versions=[1], write_version=1) if allow_legacy else None
    value = info.get('checkpoint_history', fallback)
    if type(value) is not dict or set(value) != {'read_versions', 'write_version'}:
        raise ValueError('invalid checkpoint history package metadata')
    readers, writer = value['read_versions'], value['write_version']
    if (type(readers) is not list or not readers or len(readers) > 2
            or any(type(v) is not int or v not in (1, 2) for v in readers)
            or 1 not in readers
            or len(set(readers)) != len(readers)
            or type(writer) is not int or writer not in readers):
        raise ValueError('invalid checkpoint history package metadata')
    # Linked version 2 rows still use a version 1 root layout.
    return set(readers), writer


def validate(before, after, profiles):
    old_readers, old_writer = formats(before, allow_legacy=True)
    new_readers, new_writer = formats(after)
    if new_writer not in old_readers or old_writer not in new_readers:
        raise ValueError('checkpoint history requires compatible activation and rollback readers')
    directory = profiles / '.checkpoints'
    history = directory / 'history.sqlite3'
    if directory.is_symlink() or history.is_symlink():
        raise ValueError('checkpoint history must not use symbolic links')
    if directory.exists() and not directory.is_dir():
        raise ValueError('checkpoint history directory is not a directory')
    if not history.exists():
        return
    if not directory.is_dir() or not history.is_file():
        raise ValueError('checkpoint history must be a regular file')
    try:
        with closing(sqlite3.connect(history.as_uri() + '?mode=ro', uri=True)) as db:
            versions = db.execute("SELECT DISTINCT json_type(layout, '$.version'), "
                                  "json_extract(layout, '$.version') FROM checkpoints")
            for kind, version in versions:
                if kind != 'integer' or version not in old_readers & new_readers:
                    raise ValueError('stored checkpoint version is unreadable by an activation or rollback package')
    except sqlite3.Error as error:
        raise ValueError('checkpoint history could not be inspected') from error
