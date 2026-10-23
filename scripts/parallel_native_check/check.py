import time
from . import client
from .support import error_code, require


def quiet(value, account):
    require(value['config']['eligible_accounts'] == [account] and
            value['config']['active_capacity'] == 1, 'pool_scope_changed')
    rows = value['residency']['residents']
    require(all(row['account'] == account for row in rows), 'unexpected_resident')
    return all(row['status'] == 'idle' and row['replacement'] is None and
               row['users'] == 0 and not row['uncertain'] and
               not row['close_started'] for row in rows)


def pending(stream):
    value = stream.snapshot()
    require(value['read_error'] is None, 'stream_read_error')
    require(not value['finished'] and not value['eof'] and
            value['terminal_completions'] == 0, 'stream_finished_before_disconnect')
    return value


def wait(binding, events, report, predicate, seconds):
    deadline = time.monotonic() + seconds
    while True:
        events.poll()
        report['native_completions'] = list(events.completed)
        report['native_send_invocations'] = list(events.send_invocations)
        report['native_send_snapshots'] = list(events.send_snapshots)
        report['admission_refusals'] = events.refusals
        require(events.refusals == 0, 'admission_refused')
        require(len(events.send_invocations) <= report['post_calls'], 'unexpected_native_send')
        require(len(report['samples']) < 2500, 'sample_limit')
        report['samples'].append({'at_unix': time.time(), 'values': binding.sample()})
        result = predicate()
        if result:
            return result
        require(time.monotonic() < deadline, 'native_check_timeout')
        time.sleep(0.1)


def settled(binding, events, report, count, failure):
    def ready():
        if len(events.completed) < count or len(events.send_snapshots) < count:
            return False
        require(len(events.completed) == count, 'unexpected_completion_count')
        value = events.completed[-1]
        require(value['body_failure'] == failure and value['cleanup_confirmed'], 'native_cleanup_outcome')
        require(len(events.send_snapshots) == count and
                events.send_invocations == list(range(1, count + 1)), 'native_send_count')
        snapshot = events.send_snapshots[-1]
        require(snapshot['send_invocations'] == count and snapshot['cleanup_confirmed'],
                'native_send_snapshot')
        require(value['observed_attempts'] == 1 and value['last_status'] == 200,
                'native_attempt_outcome')
        if failure is None:
            require(value['yielded_bytes'] > 0, 'native_empty_body')
        return quiet(client.pool(), binding.account)
    wait(binding, events, report, ready, 30)
    binding.refresh()


def run(binding, events, report):
    events.poll()
    require(not events.send_invocations and not events.send_snapshots and not events.completed,
            'native_work_before_test')
    require(quiet(client.pool(), binding.account), 'native_work_before_test')
    binding.refresh()
    report['stage'] = 'cancel_stream'
    stream = client.send(binding, True, report)
    try:
        def started():
            value = pending(stream)
            require(not events.completed, 'completion_before_disconnect')
            if value['text_characters'] > 0:
                require(events.send_invocations == [1], 'native_send_count')
            # This adapter renders after receiving complete upstream JSON.
            # Wait for the accepted upstream headers before interrupting.
            # A send event alone precedes headers and cannot prove acceptance.
            if events.send_invocations == [1] and value['status'] == 200:
                return value
            return None
        report['started_stream'] = wait(binding, events, report, started, 60)
        report['send_seen_at_unix'] = time.time()
        report['hello_head'] = client.head()
        pending(stream)
        events.poll()
        require(not events.completed, 'completion_before_disconnect')
        stream.close(cancel=True)
        report['cancel_stream'] = stream.snapshot()
    finally:
        stream.close()
        report['cancel_stream'] = stream.snapshot()
    report['stage'] = 'cancel_cleanup'
    settled(binding, events, report, 1, 'Cancelled')
    report['stage'] = 'complete_stream'
    stream = client.send(binding, False, report)
    try:
        def finished():
            value = stream.snapshot()
            require(value['read_error'] is None, 'stream_read_error')
            return value if value['finished'] else None
        value = wait(binding, events, report, finished, 70)
        require(value['status'] == 200 and value['eof'] and
                value['terminal_completions'] == 1 and value['text_characters'] > 0,
                'incomplete_stream_delivery')
        report['complete_stream'] = value
    finally:
        stream.close()
        report['complete_stream'] = stream.snapshot()
    report['stage'] = 'complete_cleanup'
    settled(binding, events, report, 2, None)
    events.poll()
    require(len(events.completed) == 2 and not events.pending and
            events.refusals == 0 and events.send_invocations == [1, 2] and
            len(events.send_snapshots) == 2, 'unexpected_final_events')
    require(report['post_calls'] == 2, 'post_count')
    report.update(accepted=True, stage='complete')


def observe_failed_cleanup(binding, events, report):
    """Observe already-started work; never send or retry an inference request."""
    outcome = {'confirmed': False}
    report['cleanup_after_error'] = outcome
    try:
        def ready():
            return (len(events.completed) == report['post_calls'] and
                    all(value['cleanup_confirmed'] for value in events.completed) and
                    quiet(client.pool(), binding.account))
        wait(binding, events, report, ready, 30)
        binding.refresh()
        outcome['confirmed'] = True
    except BaseException as error:
        outcome['error_code'] = error_code(error)
