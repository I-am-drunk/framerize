"""Explicit Linux migration of one authenticated legacy session. Never automatic."""
import argparse
from contextlib import contextmanager, closing
import fcntl
import hashlib
import http.client
import json
import os
from pathlib import Path
import secrets
import signal
import sqlite3
import ssl
import stat
import uuid

COOKIE_LIMIT = 16 * 1024
RESPONSE_LIMIT = 64 * 1024
FLAGS = os.O_CLOEXEC | os.O_NOFOLLOW | os.O_NONBLOCK
PLAN_KEYS = {'version', 'account_id', 'expected_user_id', 'account_sha256',
             'database', 'session_directory'}


class Refused(Exception):
    """Payload-free outcome safe for the command's public diagnostic."""


def unique_object(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise Refused('AMBIGUOUS_JSON')
        value[key] = item
    return value


def validate_plan(plan):
    if not isinstance(plan, dict) or set(plan) != PLAN_KEYS or type(plan['version']) is not int or plan['version'] != 1:
        raise Refused('INVALID_PLAN')
    if any(not isinstance(plan[key], str) for key in PLAN_KEYS - {'version'}):
        raise Refused('INVALID_PLAN')
    if str(uuid.UUID(plan['account_id'])) != plan['account_id']:
        raise Refused('INVALID_ACCOUNT_ID')
    user = plan['expected_user_id']
    if not user or any(c in user for c in '/\\\0'):
        raise Refused('INVALID_USER_ID')
    user.encode('utf-8')
    digest = plan['account_sha256']
    if len(digest) != 64 or any(c not in '0123456789abcdef' for c in digest):
        raise Refused('INVALID_ACCOUNT_BINDING')
    if any(not Path(plan[key]).is_absolute() for key in ('database', 'session_directory')):
        raise Refused('ABSOLUTE_PATH_REQUIRED')


def signature(meta):
    return (meta.st_dev, meta.st_ino, meta.st_mode, meta.st_uid,
            meta.st_size, meta.st_mtime_ns, meta.st_ctime_ns, meta.st_nlink)


def private_regular(fd):
    meta = os.fstat(fd)
    if not stat.S_ISREG(meta.st_mode) or meta.st_nlink != 1:
        raise Refused('NOT_A_PRIVATE_REGULAR_FILE')
    if meta.st_uid != os.geteuid() or meta.st_mode & 0o077:
        raise Refused('NOT_A_PRIVATE_REGULAR_FILE')
    return meta


def unchanged(directory, name, fd, before):
    current = os.stat(name, dir_fd=directory, follow_symlinks=False)
    if signature(before) != signature(os.fstat(fd)) or signature(before) != signature(current):
        raise Refused('SOURCE_BINDING_CHANGED')


@contextmanager
def owned_directory(path):
    directory = os.open(path, os.O_RDONLY | os.O_DIRECTORY | FLAGS)
    try:
        meta = os.fstat(directory)
        if meta.st_uid != os.geteuid() or meta.st_mode & 0o077:
            raise Refused('SESSION_DIRECTORY_NOT_PRIVATE')
        lock = os.open('.owner.lock', os.O_RDWR | FLAGS, dir_fd=directory)
        try:
            before = private_regular(lock)
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                raise Refused('SESSION_OWNER_BUSY') from None
            unchanged(directory, '.owner.lock', lock, before)
            yield directory, lock, before
        finally:
            os.close(lock)
    finally:
        os.close(directory)


def destination(user):
    digest = hashlib.sha256(b'framerize-session-v2\0' + user.encode()).hexdigest()
    return 'session-v2-' + digest + '.data'


def bound_account(plan):
    uri = Path(plan['database']).as_uri() + '?mode=ro'
    with closing(sqlite3.connect(uri, uri=True, timeout=2)) as db:
        db.text_factory = bytes
        db.execute('PRAGMA query_only=ON')
        row = db.execute('SELECT id,json,typeof(id),typeof(json) FROM accounts WHERE id=?',
                         (plan['account_id'],)).fetchone()
    if row is None:
        raise Refused('ACCOUNT_MISSING')
    key, raw, key_type, value_type = row
    if key_type != b'text' or value_type != b'text':
        raise Refused('ACCOUNT_ENCODING')
    account = json.loads(raw.decode('utf-8'), object_pairs_hook=unique_object)
    if key != str(uuid.UUID(account['id'])).encode() or account['framer_user_id'] != plan['expected_user_id']:
        raise Refused('ACCOUNT_IDENTITY_CHANGED')
    if hashlib.sha256(raw).hexdigest() != plan['account_sha256']:
        raise Refused('ACCOUNT_BINDING_CHANGED')
    if account.get('proxy') is not None:
        raise Refused('PROXY_ROUTE_NOT_SUPPORTED')


def absent(directory, name):
    try:
        os.stat(name, dir_fd=directory, follow_symlinks=False)
    except FileNotFoundError:
        return
    raise Refused('V2_RECORD_ALREADY_EXISTS')


def read_cookie(fd):
    before = private_regular(fd)
    if not 0 < before.st_size <= COOKIE_LIMIT:
        raise Refused('LEGACY_COOKIE_SIZE')
    value = bytearray()
    while len(value) <= COOKIE_LIMIT:
        part = os.read(fd, COOKIE_LIMIT + 1 - len(value))
        if not part:
            break
        value.extend(part)
    if not 0 < len(value) <= COOKIE_LIMIT:
        raise Refused('LEGACY_COOKIE_SIZE')
    if any(c < 32 or c > 126 for c in value):
        raise Refused('LEGACY_COOKIE_ENCODING')
    if signature(before) != signature(os.fstat(fd)):
        raise Refused('SOURCE_BINDING_CHANGED')
    return bytes(value), before


@contextmanager
def identity_deadline():
    if signal.getitimer(signal.ITIMER_REAL) != (0.0, 0.0):
        raise Refused('EXISTING_PROCESS_TIMER')
    def timeout(signum, frame):
        raise Refused('IDENTITY_REQUEST_TIMEOUT')
    prior = signal.signal(signal.SIGALRM, timeout)
    try:
        signal.setitimer(signal.ITIMER_REAL, 30)
        yield
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, prior)


def verify_identity(cookie):
    # Fixed direct HTTPS route. No environment proxy, redirect or retry support.
    connection = http.client.HTTPSConnection('api.framer.com', timeout=10,
                                              context=ssl.create_default_context())
    try:
        with identity_deadline():
            connection.request('GET', '/web/users/me', headers={
                'Cookie': cookie.decode('ascii'), 'X-Requested-By': 'Framer',
                'User-Agent': 'Framerize/0.1', 'Accept': 'application/json',
                'Connection': 'close'})
            response = connection.getresponse()
            if response.status != 200:
                raise Refused('IDENTITY_HTTP_REFUSED')
            raw = response.read(RESPONSE_LIMIT + 1)
            if len(raw) > RESPONSE_LIMIT:
                raise Refused('IDENTITY_RESPONSE_TOO_LARGE')
            result = json.loads(raw.decode('utf-8'), object_pairs_hook=unique_object)
            if not isinstance(result, dict) or not isinstance(result.get('id'), str):
                raise Refused('IDENTITY_RESPONSE_INVALID')
            return result['id']
    except Refused:
        raise
    except (OSError, ValueError, http.client.HTTPException):
        raise Refused('IDENTITY_REQUEST_FAILED') from None
    finally:
        connection.close()


def write_all(fd, value):
    while value:
        count = os.write(fd, value)
        if count <= 0:
            raise Refused('SESSION_WRITE_FAILED')
        value = value[count:]


def remove_owned_temporary(directory, name, fd):
    try:
        current = os.stat(name, dir_fd=directory, follow_symlinks=False)
    except FileNotFoundError:
        return True
    owned = os.fstat(fd)
    if (current.st_dev, current.st_ino) != (owned.st_dev, owned.st_ino):
        return False
    os.unlink(name, dir_fd=directory)
    return True


def publish(directory, name, user, cookie, validate):
    temporary = '.session-migration-' + secrets.token_hex(16) + '.tmp'
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL | FLAGS,
                 0o600, dir_fd=directory)
    linked, failure = False, None
    try:
        private_regular(fd)
        label = user.encode('utf-8')
        write_all(fd, b'FRMSESS2' + len(label).to_bytes(8, 'big') + label + cookie)
        os.fsync(fd)
        unchanged(directory, temporary, fd, os.fstat(fd))
        validate()
        os.link(temporary, name, src_dir_fd=directory, dst_dir_fd=directory,
                follow_symlinks=False)
        linked = True
        if not remove_owned_temporary(directory, temporary, fd):
            raise Refused('TEMPORARY_CLEANUP_UNCONFIRMED')
        os.fsync(directory)
    except BaseException as error:
        failure = error
    try:
        cleaned = remove_owned_temporary(directory, temporary, fd)
    except OSError:
        cleaned = False
    finally:
        os.close(fd)
    if linked and (failure is not None or not cleaned):
        raise Refused('V2_PUBLISHED_DURABILITY_UNCONFIRMED') from None
    if not cleaned:
        raise Refused('TEMPORARY_CLEANUP_UNCONFIRMED') from None
    if failure is not None:
        raise failure


def migrate(plan, verifier=verify_identity):
    validate_plan(plan)
    user = plan['expected_user_id']
    name = destination(user)
    legacy = user + '.session'
    with owned_directory(plan['session_directory']) as (directory, lock, lock_before):
        bound_account(plan)
        absent(directory, name)
        fd = os.open(legacy, os.O_RDONLY | FLAGS, dir_fd=directory)
        try:
            cookie, before = read_cookie(fd)
            unchanged(directory, legacy, fd, before)
            observed = verifier(cookie)
            if not isinstance(observed, str) or observed != user:
                raise Refused('AUTHENTICATED_USER_MISMATCH')
            def validate():
                bound_account(plan)
                unchanged(directory, legacy, fd, before)
                unchanged(directory, '.owner.lock', lock, lock_before)
                path_meta = os.stat(plan['session_directory'], follow_symlinks=False)
                held_meta = os.fstat(directory)
                if held_meta.st_uid != os.geteuid() or held_meta.st_mode & 0o077:
                    raise Refused('SESSION_DIRECTORY_NOT_PRIVATE')
                if (path_meta.st_dev, path_meta.st_ino, path_meta.st_mode) != (
                        held_meta.st_dev, held_meta.st_ino, held_meta.st_mode):
                    raise Refused('SESSION_DIRECTORY_CHANGED')
                absent(directory, name)
            publish(directory, name, user, cookie, validate)
        finally:
            os.close(fd)
    return {'status': 'V2_SESSION_PUBLISHED_IDENTITY_VERIFIED',
            'legacy_modified': False, 'account_modified': False,
            'inference_started': False, 'admission_still_required': True}


def read_plan(path):
    fd = os.open(path, os.O_RDONLY | FLAGS)
    try:
        meta = private_regular(fd)
        if not 0 < meta.st_size <= 32768:
            raise Refused('PLAN_SIZE')
        with os.fdopen(os.dup(fd), 'rb') as stream:
            raw = stream.read(32769)
        if len(raw) > 32768 or signature(meta) != signature(os.fstat(fd)):
            raise Refused('PLAN_CHANGED')
        plan = json.loads(raw, object_pairs_hook=unique_object)
        validate_plan(plan)
        return plan
    finally:
        os.close(fd)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plan', required=True, help='Private immutable JSON plan')
    parser.add_argument('--execute', action='store_true', help='Perform the separately authorized migration')
    args = parser.parse_args(argv)
    try:
        plan = read_plan(args.plan)
        result = migrate(plan) if args.execute else {
            'status': 'PLAN_VALID_NO_EXECUTION', 'migration_started': False}
        code = 0
    except Refused as error:
        reason = str(error)
        uncertain = reason in ('V2_PUBLISHED_DURABILITY_UNCONFIRMED', 'TEMPORARY_CLEANUP_UNCONFIRMED')
        result = {'status': 'UNCERTAIN' if uncertain else 'REFUSED', 'reason': reason}
        code = 3 if uncertain else 2
    except KeyboardInterrupt:
        result, code = {'status': 'CANCELLED'}, 130
    except Exception:
        result, code = {'status': 'REFUSED', 'reason': 'MIGRATION_IO_OR_DECODE_FAILED'}, 2
    print(json.dumps(result, sort_keys=True))
    return code


if __name__ == '__main__':
    raise SystemExit(main())
