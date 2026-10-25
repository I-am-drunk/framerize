"""Track account work until the actual operation, rather than its waiter, ends."""
import asyncio
from contextlib import AbstractContextManager


class AccountActivity:
    def __init__(self):
        self.activities = set()
        self.pending = set()

    def activity(self):
        activity = Activity(self)
        self.activities.add(activity)
        return activity

    def track(self, make):
        task = asyncio.create_task(make())
        self.pending.add(task)

        def complete(done):
            self.pending.discard(done)
            if not done.cancelled():
                done.exception()

        task.add_done_callback(complete)
        return task


class Activity(AbstractContextManager):
    def __init__(self, owner):
        self.owner = owner

    def __exit__(self, *_):
        self.close()

    def close(self):
        self.owner.activities.discard(self)
