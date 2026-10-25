"""Synthetic plan construction and pre-authentication refusal checks."""
import ast
import json
from pathlib import Path
import tempfile
import unittest
import uuid
from plan_policy import validate_interruption

HERE = Path(__file__).resolve().parent


def policy():
    return dict(enabled=True, control_ip='127.0.0.2', port=8787,
                lease_seconds=45, nft_sha256='a' * 64)


def extract(file, name, namespace):
    tree = ast.parse((HERE / file).read_text())
    nodes = [n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == name]
    assert len(nodes) == 1
    exec(compile(ast.Module(nodes, type_ignores=[]), file, 'exec'), namespace)
    return namespace[name]


class PlanPolicyTests(unittest.TestCase):
    def test_builder_preserves_required_interruption_policy(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            stage = root / 'package-stage-shared-v3-r1'
            stage.mkdir()
            (stage / 'inputs.json').write_text(json.dumps(dict(release='/candidate', files=[])))
            namespace = dict(BASE=root, P=root, HERE=root, Path=Path, json=json,
                             uuid=uuid, sha=lambda p: 'a' * 64,
                             configuration=lambda argv, restart: restart,
                             proposed=lambda name, text: dict(proposed_dropin=name,
                                                              dropin_sha256='a' * 64))
            make_plan = extract('prepare_shared.py', 'make_plan', namespace)
            previous = dict(argv=['/old/bin', '--browser-request-ownership',
                            '--sidecar-script', '/old/host.py'], purpose='rollout',
                            execution_lock='/fixture/lock', capture_cleanup={},
                            binary_sha256='b' * 64, sidecar='/old/host.py',
                            sidecar_sha256='c' * 64, interruption=policy())
            plan = make_plan(previous, dict(path='/fixture/request', sha256='d' * 64))
            validate_interruption(plan, lambda p: 'a' * 64)
            self.assertEqual(plan['interruption'], previous['interruption'])

    def test_missing_changed_and_mistyped_policy_is_refused(self):
        validate_interruption(dict(interruption=policy()), lambda p: 'a' * 64)
        bad = [None, {}, dict(policy(), extra=True)]
        for key in policy():
            row = policy(); del row[key]; bad.append(row)
        for key, value in dict(enabled=1, control_ip='127.0.0.1', port=True,
                               lease_seconds=45.0, nft_sha256='b' * 64).items():
            bad.append(dict(policy(), **{key: value}))
        for row in bad:
            with self.subTest(row=row), self.assertRaises(RuntimeError):
                validate_interruption(dict(interruption=row), lambda p: 'a' * 64)

    def test_controller_refuses_before_runtime_validation(self):
        def need(ok, reason):
            if not ok:
                raise RuntimeError(reason)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            plan = dict(purpose='authorized-normal-8787-rollout', ready_for_execution=True,
                        binary_sha256='6a2f0a9355f4e73f2a55846eee8bee831f4044089a05304e5405d8432ad64f11',
                        sources={})
            (root / 'plan.json').write_text(json.dumps(plan))
            calls = []
            namespace = dict(HERE=root, need=need, json=json, sha=lambda p: 'a' * 64,
                             verify_package=lambda p: calls.append('package'),
                             verify_worker=lambda p: calls.append('worker'))
            check = extract('controller.py', 'checked_plan', namespace)
            with self.assertRaisesRegex(RuntimeError, 'interruption_plan_missing'):
                check('a' * 64)
            self.assertEqual(calls, [])
            plan['interruption'] = policy()
            (root / 'plan.json').write_text(json.dumps(plan))
            self.assertEqual(check('a' * 64), plan)
            self.assertEqual(calls, ['package', 'worker'])
