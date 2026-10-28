"""Prepare or validate offline review JSON. Never run a model or publish a review."""
import argparse
import json
import os
from pathlib import Path
import stat
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from review_pilot.contracts import canonical, prepare, validate


def unique(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('duplicate JSON field')
        result[key] = value
    return result


def invalid_constant(_):
    raise ValueError('invalid JSON number')


def read(path):
    fd = os.open(path, os.O_RDONLY | getattr(os, 'O_NONBLOCK', 0) | getattr(os, 'O_NOFOLLOW', 0))
    with os.fdopen(fd, 'rb') as source:
        if not stat.S_ISREG(os.fstat(source.fileno()).st_mode):
            raise ValueError('input must be a regular file')
        data = source.read(2 * 1024 * 1024 + 1)
    if len(data) > 2 * 1024 * 1024:
        raise ValueError('input exceeds size limit')
    return json.loads(data, object_pairs_hook=unique, parse_constant=invalid_constant)


def write(path, value):
    data = canonical(value) + b'\n'
    if len(data) > 2 * 1024 * 1024:
        raise ValueError('output exceeds size limit')
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, 'wb') as output:
        output.write(data)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    planning = commands.add_parser('prepare')
    planning.add_argument('snapshot', type=Path)
    planning.add_argument('policy', type=Path)
    planning.add_argument('--head', required=True)
    planning.add_argument('--base', required=True)
    planning.add_argument('--output', type=Path, required=True)
    checking = commands.add_parser('validate')
    checking.add_argument('packet', type=Path)
    checking.add_argument('result', type=Path)
    checking.add_argument('--packet-sha256', required=True)
    checking.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.command == 'prepare':
            value = prepare(read(args.snapshot), read(args.policy), args.head, args.base)
        else:
            value = validate(read(args.packet), read(args.result), args.packet_sha256)
        write(args.output, value)
    except (ValueError, OSError, TypeError, KeyError, OverflowError, RecursionError) as error:
        parser.exit(2, f'offline review packet refused: {type(error).__name__}\n')


if __name__ == '__main__':
    main()
