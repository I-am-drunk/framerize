"""Called only by the separately bound rollout controller, never on import."""
import http.client
from control_http import connection as control_connection
import json
import time
import uuid
from criteria import ready_to_disconnect, ready_for_started_request, complete, attempts, final_counts
from event_log import EventLog
from streaming import Stream
from support import require

def send(account, model, cancel, report):
    require(report['post_calls'] < 2, 'post_limit')
    prompt = ('Count from one to five hundred in words, in order.' if cancel
              else 'Reply with exactly OK.')
    body = json.dumps(dict(model=model, input=prompt, stream=True,
                           max_output_tokens=700 if cancel else 16))
    connection = control_connection(timeout=65)
    report['post_calls'] += 1
    try:
        connection.request('POST', '/openai/v1/responses', body=body, headers={
            'Content-Type': 'application/json', 'Connection': 'close',
            'x-framerize-account': account, 'x-framerize-session': uuid.uuid4().hex})
        return Stream(connection)
    except BaseException:
        connection.close()
        raise

def observe(events, expected, identity_check):
    identity_check()
    events.poll()
    attempts(events, expected)

def within(deadline, reason):
    require(time.monotonic() < deadline, reason)
    time.sleep(0.1)

def cancel_first(stream, events, report, identity_check, head_check, *, boundary='generated_text'):
    choices = {'generated_text': ready_to_disconnect, 'request_started': ready_for_started_request}
    require(type(boundary) is str and boundary in choices, 'unknown_interruption_boundary')
    ready = choices[boundary]
    report['interruption_boundary'] = boundary
    # Best-effort start/completion logs cannot attest a pending browser fetch.
    report['upstream_cancellation_proven'] = False
    deadline = time.monotonic() + 70
    while True:
        observe(events, 1, identity_check)
        state = stream.snapshot()
        report['first_stream_at_check'] = state
        require(events.count('ai.post') == events.count('ai.chain') == 0,
                'call_finished_before_disconnect')
        if ready(state) and events.count('ai.post.started') == 1:
            break
        within(deadline, 'generated_content_not_observed' if boundary == 'generated_text'
               else 'started_request_not_observed')
    if state['text_characters'] > 0:
        report['generated_content'] = state
    report['interruption_ready_state'] = state
    root = state['operation_id']
    head_check()
    observe(events, 1, identity_check)
    state = stream.snapshot()
    report['first_stream_at_disconnect_check'] = state
    require(ready(state), 'stream_not_interruptible')
    require(events.count('ai.post') == events.count('ai.chain') == 0,
            'call_completed_during_head')
    report['text_observed_before_disconnect'] = state.get('delta_text_characters', 0) > 0
    stream.close(cancel=True)
    report['disconnected_at_unix'] = stream.snapshot()['disconnected_at_unix']
    deadline = time.monotonic() + 190
    while True:
        observe(events, 1, identity_check)
        if events.joined(root, after=report['disconnected_at_unix']):
            final_counts(events, 1, [root])
            break
        within(deadline, 'joined_cleanup_unconfirmed')
    report['joined_cleanup_at_unix'] = time.time()
    return root

def deliver_second(stream, events, report, identity_check, first_root):
    deadline = time.monotonic() + 65
    while True:
        observe(events, 2, identity_check)
        state = stream.snapshot()
        if complete(state):
            break
        within(deadline, 'response_deadline')
    root = state['operation_id']
    require(root != first_root, 'operation_identity_reused')
    report['completed_stream'] = state
    deadline = time.monotonic() + 10
    while True:
        observe(events, 2, identity_check)
        if events.joined(root):
            final_counts(events, 2, [first_root, root])
            break
        within(deadline, 'second_join_unconfirmed')
    return root

def run_pair(account, model, log, report, identity_check, quiet_check, head_check, *,
             cancellation_boundary='generated_text'):
    require(type(cancellation_boundary) is str and cancellation_boundary in
            ('generated_text', 'request_started'), 'unknown_interruption_boundary')
    require(report['post_calls'] == 0, 'acceptance_already_started')
    identity_check()
    quiet_check()
    events = EventLog(log)
    try:
        first = send(account, model, True, report)
        try:
            root = cancel_first(first, events, report, identity_check, head_check,
                                boundary=cancellation_boundary)
        finally:
            try:
                report['first_stream_before_cleanup'] = first.snapshot()
            finally:
                first.close()
        identity_check()
        quiet_check()
        second = send(account, model, False, report)
        try:
            other = deliver_second(second, events, report, identity_check, root)
            if cancellation_boundary == 'request_started':
                require(report['completed_stream'].get('delta_text_characters', 0) > 0,
                        'delivery_delta_missing')
        finally:
            second.close()
        identity_check()
        quiet_check()
        report.update(accepted=True, roots=[root, other],
                      acceptance_scope=cancellation_boundary + '_disconnect_then_delivery')
    finally:
        report['events'] = events.rows
        events.close()
