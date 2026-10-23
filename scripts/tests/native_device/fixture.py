import ast
import asyncio
import json
import tempfile
import unittest
from collections import deque
from pathlib import Path
from types import SimpleNamespace
from native_device import NativeDevices

ACCOUNT = '00000000-0000-0000-0000-000000000001'
LEASE = '00000000-0000-0000-0000-000000000002'
SECOND = '00000000-0000-0000-0000-000000000003'

SOURCE = Path(__file__).resolve().parents[2] / 'camoufox_host.py'
tree = ast.parse(SOURCE.read_text())
host_node = next(n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == 'Host')
close_node = next(n for n in host_node.body
                  if isinstance(n, ast.AsyncFunctionDef) and n.name == '_finish_close')
namespace = dict(asyncio=asyncio, emit=lambda *args, **kwargs: None)
exec(compile(ast.Module(body=[close_node], type_ignores=[]), str(SOURCE), 'exec'), namespace)
close_browser = namespace['_finish_close']

class Fixture(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        (self.root / ACCOUNT).mkdir()
        self.path = self.root / ACCOUNT / 'framerize-devices.json'
        self.path.write_text(json.dumps(['fixture-primary', 'fixture-other']))
        self.life = SimpleNamespace(phase='absent', active_commands=0,
            cooling={}, requests={}, last_mint=900.0, restored_primary_device='fixture-primary')
        self.host = SimpleNamespace(generation='a' * 32, accounts={},
            lifecycles={ACCOUNT: self.life}, _cooldown_restoration={'generation': 'a' * 32})
        self.now, self.delays = 1000.0, []
        self.api = NativeDevices(self.host, self.root, gap=2.5, window=600,
            capacity=180, cooldown=600, clock=lambda: self.now, sleep=self.sleep)
        self.request = dict(account=ACCOUNT, lease=LEASE, generation=self.host.generation,
            budget_ms=16000, sequence=1, joined=True, risk_refused=False)

    async def sleep(self, seconds):
        self.delays.append(seconds)
        self.now += seconds
        await asyncio.sleep(0)
