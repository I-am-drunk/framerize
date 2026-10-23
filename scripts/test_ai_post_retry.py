"""Exercise the real host retry loop without importing browser dependencies."""
import ast
import asyncio
from pathlib import Path
from types import SimpleNamespace
import time
import unittest
import uuid

SOURCE = Path(__file__).with_name("camoufox_host.py")
tree = ast.parse(SOURCE.read_text())
host = next(node for node in tree.body if isinstance(node, ast.ClassDef) and node.name == "Host")
method = next(node for node in host.body if isinstance(node, ast.AsyncFunctionDef) and node.name == "_handle")
namespace = dict(asyncio=asyncio, time=time, uuid=uuid,
                 trace=lambda *a, **k: None, emit=lambda *a, **k: None)
exec(compile(ast.Module(body=[method], type_ignores=[]), str(SOURCE), "exec"), namespace)

class HostFixture:
    _handle = namespace["_handle"]
    TOKEN_TTL_S = 600

    def __init__(self, expired=False):
        self.acc = SimpleNamespace(devices=["fixture"], page=object(), browser=None,
                                   token_pool={}, project=None, bearer=None)
        self.expired = expired
        self.mints = self.fetches = self.releases = self.cooled = 0
        self.started = asyncio.Event()
        self.finish = asyncio.Event()
        self.finish.set()

    async def _account(self, account): return self.acc
    def _pick_device_sticky(self, acc): return "fixture"
    async def _ensure_editor(self, *args): pass
    def _note_request(self, *args): pass
    def _cool_device(self, *args): self.cooled += 1
    def _token_release(self, *args): self.releases += 1

    async def _token_acquire(self, *args):
        self.mints += 1
        key = str(self.mints)
        self.acc.token_pool[key] = {"born": time.monotonic() - (550 if self.expired else 0)}
        return key, "fixture-token", "fixture"

    async def _run_job(self, *args):
        self.fetches += 1
        self.started.set()
        await self.finish.wait()
        return {"status": 403 if self.fetches == 1 else 200, "body": "fixture", "usage": None}

class RetryTests(unittest.IsolatedAsyncioTestCase):
    def request(self, **extra):
        return dict(cmd="ai_post", account="fixture", path="/fixture",
                    body={"request": {}}, **extra)

    async def test_deferred_retry_requires_a_second_authorized_command(self):
        for expired in (False, True):
            host = HostFixture(expired)
            host.finish.clear()
            request = asyncio.create_task(host._handle(
                self.request(defer_retries=True, attempt_index=0)))
            await asyncio.wait_for(host.started.wait(), 1)
            self.assertFalse(request.done())
            self.assertEqual((host.mints, host.fetches, host.releases), (1, 1, 0))
            host.finish.set()
            first = await asyncio.wait_for(request, 1)
            self.assertTrue(first["_retry_device"])
            self.assertEqual((host.mints, host.fetches, host.releases), (1, 1, 1))
            second = await host._handle(self.request(defer_retries=True, attempt_index=1))
            self.assertEqual(second["status"], 200)
            self.assertEqual((host.mints, host.fetches, host.releases), (2, 2, 2))

    async def test_ordinary_command_preserves_existing_internal_retry(self):
        host = HostFixture()
        result = await host._handle(self.request())
        self.assertEqual(result["status"], 200)
        self.assertEqual((host.mints, host.fetches, host.releases, host.cooled), (2, 2, 2, 1))

if __name__ == "__main__":
    unittest.main()
