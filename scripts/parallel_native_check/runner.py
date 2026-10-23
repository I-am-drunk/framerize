import argparse
import json
import os
from pathlib import Path
import time
from .check import observe_failed_cleanup, run
from .events import Events
from .guard import Binding
from .support import error_code, read_json, require, sha


def claim_attempt(binding, started):
    marker = binding.root / ('native-check-' + binding.plan['invocation_id'] + '.started')
    with marker.open('x') as stream:
        json.dump(started, stream)


def main():
    parser = argparse.ArgumentParser(description='One bounded check on an explicitly isolated native instance.')
    parser.add_argument('--plan', required=True)
    parser.add_argument('--plan-sha256', required=True)
    parser.add_argument('--report', required=True)
    parser.add_argument('--execute', action='store_true', required=True)
    args = parser.parse_args()
    os.umask(0o077)
    read_json(args.plan, digest=args.plan_sha256)
    output = Path(args.report)
    require(output.is_absolute() and output.parent.resolve(strict=True) == output.parent, 'report_path')
    require(not output.exists(), 'report_already_exists')
    started = {'started_at_unix': time.time(), 'plan_sha256': args.plan_sha256}
    with Path(str(output) + '.started').open('x') as stream:
        json.dump(started, stream)
    report = dict(started, accepted=False, post_calls=0, samples=[], native_completions=[],
                  native_send_invocations=[], native_send_snapshots=[])
    events = None
    binding = None
    try:
        binding = Binding(args.plan, args.plan_sha256)
        require(sha(args.plan) == args.plan_sha256, 'plan_changed')
        claim_attempt(binding, started)
        events = Events(binding.plan['events_path'], binding.root)
        run(binding, events, report)
    except BaseException as error:
        report['accepted'] = False
        report['error_code'] = error_code(error)
        if events is not None and report['post_calls']:
            observe_failed_cleanup(binding, events, report)
    finally:
        if events is not None:
            report['native_completions'] = list(events.completed)
            report['native_send_invocations'] = list(events.send_invocations)
            report['native_send_snapshots'] = list(events.send_snapshots)
            events.close()
    report['finished_at_unix'] = time.time()
    report['measurement_scope'] = 'Isolated service samples; memory.peak is a lifetime counter.'
    encoded = json.dumps(report, indent=2, sort_keys=True) + '\n'
    require(len(encoded.encode()) <= 1024**2, 'report_byte_limit')
    with output.open('x') as stream:
        stream.write(encoded)
    print(json.dumps({'accepted': report['accepted'], 'report': str(output),
                      'post_calls': report['post_calls'], 'error_code': report.get('error_code')}))
    return 0 if report['accepted'] else 1
