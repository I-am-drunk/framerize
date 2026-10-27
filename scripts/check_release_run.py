"""Refuse PR, skipped, failed and unrelated CI runs as release evidence."""
import json
import os
import re
import subprocess
import sys


def validate(run, jobs):
    if (run.get('repository', {}).get('full_name') != 'I-am-drunk/framerize'
            or run.get('path') != '.github/workflows/ci.yml'
            or run.get('event') not in ['push', 'workflow_dispatch']
            or run.get('head_branch') not in ['main', 'browser-based', 'browserless']
            or run.get('status') != 'completed' or run.get('conclusion') != 'success'
            or not re.fullmatch('[0-9a-f]{40}', run.get('head_sha', ''))):
        raise ValueError('release requires successful canonical maintained-branch CI')
    outcomes = {job['name']: job.get('conclusion') for job in jobs}
    for name in ['build (framerize)', 'build (framerize-native)', 'Check']:
        if outcomes.get(name) != 'success':
            raise ValueError('required CI job did not pass: ' + name)
    return run['head_sha']


if __name__ == '__main__':
    if len(sys.argv) != 2 or not re.fullmatch('[0-9]+', sys.argv[1]):
        raise SystemExit('numeric CI run ID required')
    endpoint = 'repos/I-am-drunk/framerize/actions/runs/' + sys.argv[1]
    def get(path):
        return json.loads(subprocess.check_output(['gh', 'api', path], text=True, timeout=30))
    run, jobs = get(endpoint), get(endpoint + '/jobs?filter=latest&per_page=100')
    if jobs['total_count'] != len(jobs['jobs']):
        raise SystemExit('incomplete CI job evidence')
    sha = validate(run, jobs['jobs'])
    with open(os.environ['GITHUB_OUTPUT'], 'a') as output:
        output.write('sha=' + sha + '\n')
