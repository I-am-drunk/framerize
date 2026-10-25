"""Private standard-I/O process for the native application's durable ledger."""
import argparse
import asyncio
from pathlib import Path
import json
import sys

from framerize_runtime.release import verify_runtime
from framerize_runtime.wire import AsyncReplyWriter
from .owner import LedgerOwner, OwnershipError, require
from .protocol import Protocol


async def serve(owner, reader, writer):
    protocol, pending = Protocol(owner), set()
    last_id, shutdown = 0, None

    async def answer(request):
        try:
            value = await protocol.call(request['op'], request['args'])
            reply = {'id': request['id'], 'ok': True, 'result': value}
        except (OwnershipError, ValueError, TypeError, KeyError):
            reply = {'id': request['id'], 'ok': False, 'error': 'rejected'}
        except Exception:
            reply = {'id': request['id'], 'ok': False, 'error': 'unconfirmed'}
        await writer.send(reply)

    await writer.send({'ready': 1})
    async with asyncio.TaskGroup() as group:
        while line := await reader.readline():
            require(len(line) <= 4096 and line.endswith(b'\n'), 'invalid ledger frame')
            request = json.loads(line)
            require(type(request) is dict and set(request) == {'id', 'op', 'args'}, 'invalid request')
            ident = request['id']
            require(type(ident) is int and last_id < ident <= 2**64 - 1, 'invalid request sequence')
            last_id = ident
            if request['op'] == 'shutdown':
                require(type(request['args']) is dict and not request['args'], 'invalid shutdown')
                shutdown = ident
                break
            if len(pending) >= 64:
                await writer.send({'id': ident, 'ok': False, 'error': 'rejected'})
                continue
            task = group.create_task(answer(request))
            pending.add(task)
            task.add_done_callback(pending.discard)
    clean = owner.shutdown()
    if shutdown is not None:
        await writer.send({'id': shutdown, 'ok': True, 'result': {'clean': clean}})
    return 0 if clean and shutdown is not None else 1


async def run(owner):
    reader = asyncio.StreamReader(limit=4096)
    transport, _ = await asyncio.get_running_loop().connect_read_pipe(
        lambda: asyncio.StreamReaderProtocol(reader), sys.stdin.buffer)
    writer = AsyncReplyWriter(sys.stdout)
    try:
        return await serve(owner, reader, writer)
    finally:
        transport.close()
        await writer.close()
        owner.shutdown()


def main(script):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', required=True, type=Path)
    args = parser.parse_args()
    try:
        verify_runtime(Path(script).resolve().parent)
        owner = LedgerOwner(args.root)
    except Exception:
        print(json.dumps({'ready': 0}), flush=True)
        return 1
    try:
        return asyncio.run(run(owner))
    except (Exception, KeyboardInterrupt):
        return 1
    finally:
        owner.shutdown()
