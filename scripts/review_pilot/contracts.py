"""Offline packet checks only. These functions grant no runtime authority."""
import hashlib
import json
from pathlib import PurePosixPath
import re

MODEL = 'gpt-6-astra-ultrafast'
IMAGE = 'pragent/pr-agent@sha256:10a8893d5e081be1a5ae825843d9fc14f8e8d59d8a4f61d67c1775415156079f'
REPOSITORY = 'I-am-drunk/framerize'
# PR-Agent 0.49.0 raises smaller output caps inside LiteLLMAIHandler.
MIN_OUTPUT_TOKENS = 4096


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True, allow_nan=False).encode()


def digest(value):
    return hashlib.sha256(canonical(value)).hexdigest()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def exact(value, fields):
    require(type(value) is dict and set(value) == set(fields.split()), 'unexpected object fields')


def text(value, limit):
    require(type(value) is str and 0 < len(value) <= limit, 'invalid text length')
    require(not any(ord(c) < 32 or 127 <= ord(c) <= 159 for c in value), 'control character in text')
    return value


def path(value):
    text(value, 512)
    require(not value.startswith('/') and '\\' not in value, 'invalid repository path')
    require(all(p not in ('', '.', '..') for p in value.split('/')), 'invalid repository path')
    return PurePosixPath(value).as_posix()


def policy(value):
    exact(value, 'model effort max_input_tokens max_output_tokens')
    require(value['model'] == MODEL, 'unexpected model')
    require(value['effort'] in ('low', 'medium', 'high', 'xhigh', 'max', 'ultra'), 'explicit effort required')
    for field in ('max_input_tokens', 'max_output_tokens'):
        require(type(value[field]) is int and 0 < value[field] <= 1000000, 'explicit positive token limit required')
    require(value['max_output_tokens'] >= MIN_OUTPUT_TOKENS,
            f'PR-Agent 0.49.0 requires max_output_tokens of at least {MIN_OUTPUT_TOKENS}')
    return dict(value)


def sha(value):
    require(type(value) is str and re.fullmatch('[0-9a-f]{40}', value), 'full commit required')
    return value


def anchors(patch, status='modified'):
    require(type(patch) is str and len(patch.encode()) <= 1024 * 1024, 'missing or oversized diff')
    old = new = 1
    old_left = new_left = added = removed = 0
    result = {'LEFT': [], 'RIGHT': []}
    active = False
    lines = patch.split('\n')
    if lines[-1] == '':
        lines.pop()
    for line in lines:
        match = re.fullmatch(r'@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@.*', line)
        if match:
            require(old_left == new_left == 0, 'truncated diff hunk')
            old_start, old_count, new_start, new_count = match.groups()
            old_start, new_start = int(old_start), int(new_start)
            old_left, new_left = int(old_count or 1), int(new_count or 1)
            if status == 'added':
                require(not active and old_start == old_left == 0 and new_start == 1 and new_left > 0,
                        'invalid added file range')
            if status == 'removed':
                require(not active and new_start == new_left == 0 and old_start == 1 and old_left > 0,
                        'invalid removed file range')
            # An empty range names the line before its insertion point.
            # Normalize to the next 1-based line before comparing consumed spans.
            old_start += old_left == 0
            new_start += new_left == 0
            require(old_start >= old and new_start >= new, 'overlapping diff hunks')
            require(old_start - old == new_start - new, 'inconsistent unchanged span')
            old, new = old_start, new_start
            active = True
            continue
        require(active and line, 'invalid diff line')
        if line == '\\ No newline at end of file':
            continue
        if status in ('added', 'removed'):
            require(line[0] == ('+' if status == 'added' else '-'), f'invalid {status} file line')
        if line[0] in ' -':
            require(old_left > 0 and old > 0, 'invalid old diff range')
            if line[0] == '-':
                result['LEFT'].append(old)
                removed += 1
            old += 1
            old_left -= 1
        if line[0] in ' +':
            require(new_left > 0 and new > 0, 'invalid new diff range')
            if line[0] == '+':
                result['RIGHT'].append(new)
                added += 1
            new += 1
            new_left -= 1
        require(line[0] in ' +-', 'invalid diff marker')
    require(old_left == new_left == 0, 'truncated diff hunk')
    require(all(len(v) == len(set(v)) for v in result.values()), 'overlapping diff hunks')
    return result, added, removed


def prepare(snapshot, settings, expected_head, expected_base):
    exact(snapshot, 'pull files')
    settings = policy(settings)
    pull, files = snapshot['pull'], snapshot['files']
    exact(pull, 'repository number state head base changed_files additions deletions')
    require(pull['repository'] == REPOSITORY and pull['state'] == 'open', 'wrong repository or closed PR')
    require(type(pull['number']) is int and pull['number'] > 0, 'invalid PR number')
    require(sha(pull['head']) == sha(expected_head) and sha(pull['base']) == sha(expected_base), 'stale PR snapshot')
    require(type(files) is list and 0 < len(files) <= 250, 'invalid file count')
    require(type(pull['changed_files']) is int and len(files) == pull['changed_files'], 'incomplete file inventory')
    prepared, seen = [], set()
    for file in files:
        exact(file, 'path previous_path status additions deletions patch')
        name = path(file['path'])
        require(name not in seen, 'duplicate changed path')
        seen.add(name)
        require(file['status'] in ('added', 'removed', 'modified', 'renamed'), 'unsupported file status')
        previous = path(file['previous_path']) if file['status'] == 'renamed' else None
        require(previous != name, 'rename must change the path')
        require(file['status'] == 'renamed' or file['previous_path'] is None, 'unexpected previous path')
        positions, added, removed = anchors(file['patch'], file['status'])
        require(type(file['additions']) is int and type(file['deletions']) is int, 'invalid change counts')
        require(added == file['additions'] and removed == file['deletions'], 'incomplete diff')
        require(added + removed > 0 or file['status'] == 'renamed', 'unreviewable file change')
        prepared.append({**file, 'previous_path': previous, 'anchors': positions})
    for field in ('additions', 'deletions'):
        require(type(pull[field]) is int and pull[field] == sum(f[field] for f in files), 'incomplete change inventory')
    packet = {'kind': 'offline-review-packet', 'pull': pull, 'files': prepared,
              'policy': settings, 'policy_sha256': digest(settings), 'image': IMAGE,
              'requested_settings': pr_agent_settings(settings),
              'runtime_verified': False, 'authority': 'none'}
    require(len(canonical(packet)) <= 2 * 1024 * 1024, 'oversized review packet')
    return {'packet': packet, 'sha256': digest(packet)}


def validate(envelope, result, expected_packet_sha256):
    exact(envelope, 'packet sha256')
    packet = envelope['packet']
    require(digest(packet) == envelope['sha256'] == expected_packet_sha256, 'review packet identity differs')
    exact(result, 'packet_sha256 model effort completed covered_files findings')
    require(result['packet_sha256'] == expected_packet_sha256, 'stale review result')
    require(result['model'] == packet['policy']['model'] and result['effort'] == packet['policy']['effort'], 'model or effort differs')
    require(result['completed'] is True, 'incomplete review result')
    coverage = result['covered_files']
    files = {f['path']: f for f in packet['files']}
    require(type(coverage) is list and all(type(p) is str for p in coverage), 'invalid coverage declaration')
    require(len(coverage) == len(files) and set(coverage) == set(files), 'incomplete or duplicate coverage')
    require(type(result['findings']) is list and len(result['findings']) <= 50, 'invalid findings count')
    findings, seen = [], set()
    for finding in result['findings']:
        exact(finding, 'path side line severity problem evidence correction')
        file = files.get(path(finding['path']))
        require(file is not None and finding['side'] in ('LEFT', 'RIGHT'), 'invalid finding target')
        require(type(finding['line']) is int and finding['line'] in file['anchors'][finding['side']], 'finding is not on a changed line')
        require(finding['severity'] in ('P1', 'P2', 'P3'), 'invalid severity')
        for field in ('problem', 'evidence', 'correction'):
            text(finding[field], 2000)
        fingerprint = digest({'packet': expected_packet_sha256, **finding})
        require(fingerprint not in seen, 'duplicate finding')
        seen.add(fingerprint)
        findings.append({**finding, 'fingerprint': fingerprint})
    return {'kind': 'offline-contract-validation', 'packet_sha256': expected_packet_sha256,
            'runtime_verified': False, 'authority': 'none', 'findings': findings}


def pr_agent_settings(value):
    settings = policy(value)
    return {'config': {'model': 'openai/' + settings['model'],
        'model_weak': 'openai/' + settings['model'], 'model_reasoning': 'openai/' + settings['model'],
        'fallback_models': [], 'reasoning_effort': settings['effort'],
        'custom_model_max_tokens': settings['max_input_tokens'], 'max_model_tokens': settings['max_input_tokens'],
        'max_output_tokens': settings['max_output_tokens'], 'publish_output': False,
        'publish_output_progress': False, 'use_repo_settings_file': False,
        'use_global_settings_file': False, 'use_group_settings_file': False,
        'enable_per_directory_settings': False, 'repo_context_files': [], 'num_retries': 0,
        'additional_reasoning_effort_models': [MODEL], 'no_temperature_models': [MODEL],
        'retry_same_model_on_timeout': False, 'retry_same_model_on_length': False,
        'propagate_tool_errors': True, 'ai_timeout': 120, 'log_level': 'WARNING'},
        'model_routing': {'enable': False, 'rules': []},
        'pr_code_suggestions': {'parallel_calls': False},
        'litellm': {'enable_callbacks': False, 'turn_off_message_logging': True}}
