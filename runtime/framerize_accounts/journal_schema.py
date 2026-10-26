"""Recognize supported schemas without deleting or guessing at stored history."""


def schema_version(db, version):
    objects = dict(db.execute("SELECT name, type FROM sqlite_master "
                              "WHERE name NOT GLOB 'sqlite_*'"))
    if version == 0 and not objects:
        return 0
    valid = version in (0, 1, 2) and objects.get('accounts') == 'table'
    valid = valid and set(objects) <= {'accounts', 'migration_evidence'}
    valid = valid and all(kind == 'table' for kind in objects.values())
    if 'migration_evidence' in objects:
        evidence = [tuple(row[1:4]) + (row[5],)
                    for row in db.execute('PRAGMA table_info(migration_evidence)')]
        valid = valid and evidence == [('record', 'TEXT', 1, 0)]
    columns = [tuple(row[1:4]) + (row[5],)
               for row in db.execute('PRAGMA table_info(accounts)')]
    base = [('account', 'TEXT', 0, 1), ('snapshot', 'TEXT', 1, 0)]
    shape = 1 if columns == base else 2 if columns == base + [
        ('runtime_owner', 'TEXT', 0, 0)] else None
    if not valid or shape is None or version not in (0, shape):
        raise RuntimeError('unsupported rate-history schema; preserve the database and restore a compatible backup')
    return shape
