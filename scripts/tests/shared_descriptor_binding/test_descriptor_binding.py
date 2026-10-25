"""Protect the observed old-release binding failure without OS access."""
import ast
from pathlib import Path
from types import SimpleNamespace
import unittest

HERE = Path(__file__).resolve().parent


def fixture(valid_source=True):
    module = SimpleNamespace(OLD_BINARY='historical', OLD_SCRIPT='historical',
                             SIDECAR=Path('/historical'))
    loader = SimpleNamespace(exec_module=lambda value: None)
    util = SimpleNamespace(spec_from_file_location=lambda *a: SimpleNamespace(loader=loader),
                           module_from_spec=lambda spec: module)
    def need(value, reason):
        if not value:
            raise RuntimeError(reason)
    namespace = dict(need=need, sha=lambda p: 'bound' if valid_source else 'changed',
                     SOURCE=Path('/fixture/protocol.py'), SOURCE_SHA='bound', Path=Path,
                     importlib=SimpleNamespace(util=util), read_request=object())
    tree = ast.parse((HERE / 'descriptor.py').read_text())
    nodes = [n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == 'bridge']
    assert len(nodes) == 1
    exec(compile(ast.Module(nodes, type_ignores=[]), 'descriptor.py', 'exec'), namespace)
    plan = dict(binary_sha256='candidate-binary', sidecar_sha256='candidate-host',
                sidecar='/candidate/host.py', rollback=dict(binary_sha256='current-binary',
                sidecar_sha256='current-host', sidecar='/current/host.py'))
    return namespace['bridge'], module, plan


class DescriptorBindingTests(unittest.TestCase):
    def test_activation_binds_current_release(self):
        bridge, module, plan = fixture()
        self.assertIs(bridge(plan, 'activate'), module)
        self.assertEqual(module.OLD_BINARY, plan['rollback']['binary_sha256'])
        self.assertEqual(module.OLD_SCRIPT, plan['rollback']['sidecar_sha256'])
        self.assertEqual(module.SIDECAR, Path(plan['rollback']['sidecar']))
        self.assertEqual(module.NEW_BINARY, plan['binary_sha256'])
        self.assertEqual(module.NEW_SCRIPT, plan['sidecar_sha256'])

    def test_rollback_binds_candidate_release(self):
        bridge, module, plan = fixture()
        bridge(plan, 'rollback')
        self.assertEqual(module.NEW_BINARY, plan['binary_sha256'])
        self.assertEqual(module.NEW_SCRIPT, plan['sidecar_sha256'])
        self.assertEqual(module.SIDECAR, Path(plan['sidecar']))

    def test_changed_protocol_is_refused_before_rebinding(self):
        bridge, module, plan = fixture(False)
        with self.assertRaisesRegex(RuntimeError, 'descriptor_source_changed'):
            bridge(plan, 'activate')
        self.assertEqual(module.OLD_BINARY, 'historical')
        self.assertFalse(hasattr(module, 'NEW_BINARY'))
