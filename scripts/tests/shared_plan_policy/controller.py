def checked_plan(digest):
    from plan_policy import validate_interruption
    path = HERE/'plan.json'
    need(sha(path) == digest, 'plan_changed')
    plan = json.loads(path.read_text())
    need(plan['purpose'] == 'authorized-normal-8787-rollout', 'wrong_purpose')
    need(plan['ready_for_execution'] is True, 'controller_preparation_incomplete')
    need(plan['binary_sha256'] ==
         '6a2f0a9355f4e73f2a55846eee8bee831f4044089a05304e5405d8432ad64f11', 'wrong_binary')
    for path, expected in plan['sources'].items():
        need(sha(path) == expected, 'controller_source_changed')
    validate_interruption(plan, sha)
    verify_package(plan)
    verify_worker(plan)
    return plan
