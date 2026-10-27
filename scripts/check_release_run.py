"""Bind release inputs to one successful CI attempt and immutable artifact IDs."""
import json
import os
from pathlib import Path
import re
import subprocess
import sys
from datetime import datetime

REPO = 'I-am-drunk/framerize'
APPS = ('framerize', 'framerize-native')


def get(path):
    return json.loads(subprocess.check_output(['gh', 'api', path], text=True, timeout=30))


def validate(run, jobs):
    if (run.get('repository', {}).get('full_name') != REPO
            or run.get('path') != '.github/workflows/ci.yml'
            or run.get('event') not in ['push', 'workflow_dispatch']
            or run.get('head_branch') not in ['main', 'browser-based', 'browserless']
            or run.get('status') != 'completed' or run.get('conclusion') != 'success'
            or not re.fullmatch('[0-9a-f]{40}', run.get('head_sha', ''))
            or type(run.get('id')) is not int or run['id'] <= 0
            or type(run.get('run_attempt')) is not int or run['run_attempt'] <= 0):
        raise ValueError('release requires successful canonical maintained-branch CI')
    for name in [*(f'build ({app})' for app in APPS), 'Check']:
        matches = [job for job in jobs if job.get('name') == name]
        if len(matches) != 1:
            raise ValueError('required CI job missing or duplicated: ' + name)
        job = matches[0]
        if (job.get('status') != 'completed' or job.get('conclusion') != 'success'
                or job.get('run_id') != run['id'] or job.get('run_attempt') != run['run_attempt']
                or job.get('head_sha') != run['head_sha'] or type(job.get('id')) is not int):
            raise ValueError('required CI job did not pass in this attempt: ' + name)
    return run['head_sha']


def bind_artifacts(run, jobs, artifacts):
    validate(run, jobs)
    selected = []
    for app in APPS:
        name = f"{app}-linux-x86_64-{run['head_sha']}-run-{run['id']}-attempt-{run['run_attempt']}"
        matches = [a for a in artifacts if a.get('name') == name]
        if len(matches) != 1:
            raise ValueError('exactly one attempt-specific artifact required: ' + app)
        artifact = matches[0]
        job = next(j for j in jobs if j['name'] == f'build ({app})')
        workflow = artifact.get('workflow_run', {})
        if (artifact.get('expired') is not False or type(artifact.get('id')) is not int
                or artifact['id'] <= 0 or workflow.get('id') != run['id']
                or workflow.get('head_sha') != run['head_sha']
                or not isinstance(artifact.get('digest'), str)
                or not re.fullmatch('sha256:[0-9a-f]{64}', artifact.get('digest', ''))):
            raise ValueError('invalid artifact identity or digest')
        times = [datetime.fromisoformat(x.replace('Z', '+00:00')) for x in
                 [job['started_at'], artifact['created_at'], job['completed_at']]]
        if not times[0] <= times[1] <= times[2]:
            raise ValueError('artifact created outside selected job attempt')
        selected.append({'application': app, 'job_id': job['id'], **artifact})
    return selected


def snapshot(run_id):
    endpoint = f'repos/{REPO}/actions/runs/{run_id}'
    run = get(endpoint)
    if run.get('id') != run_id:
        raise ValueError('CI run ID differs')
    jobs = get(endpoint + f"/attempts/{run['run_attempt']}/jobs?per_page=100")
    artifacts = get(endpoint + '/artifacts?per_page=100')
    if jobs['total_count'] != len(jobs['jobs']) or artifacts['total_count'] != len(artifacts['artifacts']):
        raise ValueError('incomplete CI evidence')
    bound = bind_artifacts(run, jobs['jobs'], artifacts['artifacts'])
    if get(endpoint) != run:
        raise ValueError('CI run changed during inspection')
    return {'run': run, 'jobs': jobs['jobs'], 'artifacts': bound}


if __name__ == '__main__':
    if len(sys.argv) != 2 or not re.fullmatch('[1-9][0-9]*', sys.argv[1]):
        raise SystemExit('positive numeric CI run ID required')
    evidence = snapshot(int(sys.argv[1]))
    Path('ci-evidence.json').write_text(json.dumps(evidence, indent=2) + '\n')
    with open(os.environ['GITHUB_OUTPUT'], 'a') as output:
        output.write('sha=' + evidence['run']['head_sha'] + '\n')
