"""Refuse an incomplete interruption binding before authentication or launch."""


def validate_interruption(plan, sha):
    expected = dict(enabled=True, control_ip='127.0.0.2', port=8787,
                    lease_seconds=45, nft_sha256=sha('/usr/sbin/nft'))
    actual = plan.get('interruption')
    if type(actual) is not dict or set(actual) != set(expected):
        raise RuntimeError('interruption_plan_missing')
    if any(type(actual[k]) is not type(v) or actual[k] != v
           for k, v in expected.items()):
        raise RuntimeError('interruption_plan_changed')
