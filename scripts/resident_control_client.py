"""One private resident-control request; no retries, credential export or deployment.

The socket must belong to an owner started with resident controls enabled.
Exit 0 means a validated report arrived, not successful inference. Exit 2
means no request was started; exit 3 means settlement remains unknown here.
Closing this client requests cancellation; the daemon retains actual work.
"""
import argparse
import json
import os
import socket
import stat
import struct
import time

MAX_PAYLOAD = 96 * 1024 - 4
INPUT = frozenset(('account_id', 'user_id', 'team_id', 'project_id',
    'conversation_id', 'route', 'feature_category', 'feature_name', 'step',
    'turn_id', 'baseline_turn_id', 'user_prompt', 'timeout_ms', 'token_limit', 'request'))
BOOLEANS = frozenset(('baseline_accepted', 'baseline_body_complete',
    'baseline_json_object', 'native_invoked', 'body_complete', 'json_object',
    'native_output_complete', 'cleanup_confirmed', 'host_fenced'))
ENUMS = {'failure': frozenset(('missing_owner', 'admission', 'context', 'acquire',
    'bearer', 'baseline', 'mint', 'native', 'cancelled', 'deadline', 'publication',
    'cleanup', 'owner_lost')), 'refusal_policy': frozenset(('castle', 'upstream')),
    'refusal_basis': frozenset(('provider_marker', 'framer_security_envelope',
        'framer_marker', 'elapsed_fallback'))}
STATUSES = frozenset(('baseline_status', 'status'))


def refused():
    raise ValueError('private resident control refused') from None


def strict_json(data):
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                refused()
            result[key] = value
        return result
    return json.loads(data.decode('utf-8', 'strict'), object_pairs_hook=pairs,
                      parse_constant=lambda _: refused())


def checked_report(data):
    value = strict_json(data)
    if type(value) is not dict or set(value) != BOOLEANS | STATUSES | ENUMS.keys():
        refused()
    for key in BOOLEANS:
        if type(value[key]) is not bool:
            refused()
    for key in STATUSES:
        if value[key] is not None and (type(value[key]) is not int or not 100 <= value[key] <= 599):
            refused()
    for key, choices in ENUMS.items():
        if value[key] is not None and (type(value[key]) is not str or value[key] not in choices):
            refused()
    if value['native_output_complete'] and not (value['native_invoked']
            and value['status'] == 200 and value['body_complete'] and value['json_object']):
        refused()
    return value


def checked_request(data):
    if not 0 < len(data) <= MAX_PAYLOAD:
        refused()
    value = strict_json(data)
    if type(value) is not dict or set(value) != INPUT or type(value['request']) is not dict:
        refused()
    secret_keys = frozenset(('authorization', 'authentication', 'auth', 'bearer',
        'cookie', 'cookies', 'cookieheader', 'setcookie', 'token', 'accesstoken',
        'refreshtoken', 'idtoken', 'castletoken', 'apikey', 'secret', 'clientsecret',
        'password', 'credentials', 'headers', 'proxy', 'proxyurl'))
    def visit(item, depth=0):
        if depth > 64:
            refused()
        if type(item) is dict:
            for key, child in item.items():
                normalized = ''.join(c.lower() for c in key if c.isascii() and c.isalnum())
                if normalized in secret_keys:
                    refused()
                visit(child, depth + 1)
        elif type(item) is list:
            for child in item:
                visit(child, depth + 1)
    visit(value)
    return struct.pack('!I', len(data)) + data


def private_request(path):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_CLOEXEC | os.O_NONBLOCK)
    with os.fdopen(fd, 'rb') as source:
        info = os.fstat(source.fileno())
        if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.geteuid()
                or stat.S_IMODE(info.st_mode) != 0o600 or info.st_size > MAX_PAYLOAD):
            refused()
        return checked_request(source.read(MAX_PAYLOAD + 1))


def socket_identity(path):
    if not os.path.isabs(path) or os.path.normpath(path) != path:
        refused()
    parent = os.path.dirname(path)
    if os.path.realpath(parent) != parent:
        refused()
    directory, node = os.stat(parent), os.lstat(path)
    if (not stat.S_ISDIR(directory.st_mode) or directory.st_uid != os.geteuid()
            or stat.S_IMODE(directory.st_mode) != 0o700
            or not stat.S_ISSOCK(node.st_mode) or node.st_uid != os.geteuid()
            or stat.S_IMODE(node.st_mode) != 0o600):
        refused()
    return directory.st_dev, directory.st_ino, node.st_dev, node.st_ino


def remaining(peer, deadline):
    budget = deadline - time.monotonic()
    if budget <= 0:
        raise TimeoutError('resident control deadline')
    peer.settimeout(budget)


def exact(peer, size, deadline):
    data = bytearray()
    while len(data) < size:
        remaining(peer, deadline)
        chunk = peer.recv(size - len(data))
        if not chunk:
            refused()
        data.extend(chunk)
    return bytes(data)


def exchange(peer, frame, deadline):
    remaining(peer, deadline)
    peer.sendall(frame)
    size = struct.unpack('!I', exact(peer, 4, deadline))[0]
    if not 0 < size <= MAX_PAYLOAD:
        refused()
    report = checked_report(exact(peer, size, deadline))
    remaining(peer, deadline)
    if peer.recv(1):
        refused()
    return report


def run(path, request, wait_seconds):
    started = False
    try:
        if type(wait_seconds) is not int or not 1 <= wait_seconds <= 180:
            refused()
        frame = private_request(request)
        identity = socket_identity(path)
        deadline = time.monotonic() + wait_seconds
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as peer:
            remaining(peer, deadline)
            peer.connect(path)
            credentials = peer.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, 12)
            _, uid, _ = struct.unpack('3i', credentials)
            if uid != os.geteuid() or socket_identity(path) != identity:
                refused()
            started = True  # A partial send is also an unknown submitted request.
            report = exchange(peer, frame, deadline)
        return {'transport': 'reported', 'report': report}, 0
    except (Exception, KeyboardInterrupt):
        return {'transport': 'unknown' if started else 'not_started', 'report': None}, 3 if started else 2


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--socket', required=True)
    parser.add_argument('--request', required=True, help='owner-only 0600 JSON input file')
    parser.add_argument('--wait-seconds', type=int, default=120)
    args = parser.parse_args()
    result, code = run(args.socket, args.request, args.wait_seconds)
    print(json.dumps(result, separators=(',', ':')))
    return code


if __name__ == '__main__':
    raise SystemExit(main())
