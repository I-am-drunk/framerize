from evidence import root_id
from support import require

def ready_for_started_request(state):
    require(state['read_error'] is None, 'cancel_stream_error')
    require(not state['finished'] and not state['eof'] and
            state['terminal_completions'] == 0, 'stream_finished_before_disconnect')
    if state['status'] is None:
        return False
    require(state['status'] == 200, 'cancel_stream_status')
    root_id(state.get('operation_id'))
    return True

def ready_to_disconnect(state):
    return ready_for_started_request(state) and state['text_characters'] > 0

def complete(state):
    require(state['read_error'] is None, 'delivery_stream_error')
    if not state['finished']:
        return False
    root_id(state.get('operation_id'))
    require(state['status'] == 200 and state['eof'] and
            state['terminal_completions'] == 1 and state['text_characters'] > 0,
            'normal_stream_incomplete')
    return True

def attempts(events, expected):
    require(events.count('ai.post.started') <= expected, 'extra_inference_attempt')
    require(not events.count('ai.chain.repick'), 'unexpected_repick')
    require(not any(r.get('metered_chunks', 0) for r in events.rows), 'metered_call')

def final_counts(events, expected, roots):
    attempts(events, expected)
    require(all(events.count(kind) == expected for kind in
                ('ai.post.started', 'ai.post', 'ai.chain')), 'inference_count_mismatch')
    chain_roots = [r.get('request_id') for r in events.rows if r['event'] == 'ai.chain']
    require(sorted(chain_roots) == sorted(roots), 'chain_identity_mismatch')
