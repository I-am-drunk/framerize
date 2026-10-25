"""Private activity ownership for a future resident-browser control.

This ledger does not select pages, start browsers, mint, or prove remote
settlement. Callers must register the actual owned RPC, not a timeout wrapper.
"""
import asyncio
import uuid
from dataclasses import dataclass


class ResidentRefused(RuntimeError):
    pass


@dataclass(frozen=True)
class Settlement:
    state: str
    browser_settled: bool


class ActivityLedger:
    def __init__(self, capacity=256):
        if type(capacity) is not int or not 1 <= capacity <= 1024:
            raise ValueError('invalid resident ledger capacity')
        self.capacity = capacity
        self.activities = set()
        self.pending = set()
        self.leases = {}
        self.current = None
        self.fenced = False

    def activity(self):
        if self.fenced or self.current is not None:
            raise ResidentRefused('resident account reserved')
        activity = Activity(self)
        self.activities.add(activity)
        return activity

    def acquire(self, operation):
        if type(operation) is not str:
            raise ResidentRefused('invalid resident operation')
        try:
            valid = str(uuid.UUID(operation)) == operation
        except ValueError:
            valid = False
        if not valid:
            raise ResidentRefused('invalid resident operation')
        if operation in self.leases:
            raise ResidentRefused('resident operation already used')
        if len(self.leases) >= self.capacity:
            raise ResidentRefused('resident operation capacity exhausted')
        if self.fenced or self.current is not None or self.activities or self.pending:
            raise ResidentRefused('resident account busy or unconfirmed')
        lease = Lease(self, operation)
        self.leases[operation] = lease
        self.current = lease
        return lease

    def track(self, make):
        task = asyncio.create_task(make())
        self.pending.add(task)

        def complete(done):
            self.pending.discard(done)
            if done.cancelled():
                self.fenced = True
            else:
                done.exception()

        task.add_done_callback(complete)
        return task


class Activity:
    def __init__(self, ledger):
        self.ledger = ledger
        self.closed = False

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()

    def close(self):
        if not self.closed:
            self.closed = True
            self.ledger.activities.remove(self)

    async def rpc(self, make):
        if self.closed or self.ledger.fenced:
            raise ResidentRefused('resident activity closed')
        return await asyncio.shield(self.ledger.track(make))


class Lease:
    def __init__(self, ledger, operation):
        self.ledger = ledger
        self.operation = operation
        self.completion = None

    async def rpc(self, make):
        if (self.completion is not None or self.ledger.fenced
                or self.ledger.current is not self):
            raise ResidentRefused('resident mint admission closed')
        return await asyncio.shield(self.ledger.track(make))

    async def settle(self, native_state, resume):
        if (type(native_state) is not str or type(resume) is not bool
                or native_state not in ('settled', 'not_started', 'unknown')):
            raise ResidentRefused('invalid resident settlement')
        if self.completion is None:
            if native_state == 'unknown' or not resume:
                self.ledger.fenced = True
            self.completion = asyncio.create_task(self._complete())
        return await asyncio.shield(self.completion)

    async def _complete(self):
        ledger = self.ledger
        try:
            pending = tuple(ledger.pending)
            await asyncio.gather(*(asyncio.shield(task) for task in pending),
                                 return_exceptions=True)
            settled = all(task.done() for task in ledger.pending)
            if ledger.current is not self or not settled:
                ledger.fenced = True
            released = not ledger.fenced
            if released:
                ledger.current = None
            return Settlement('released' if released else 'fenced', settled)
        except BaseException:
            ledger.fenced = True
            raise
