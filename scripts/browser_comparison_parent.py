"""Finite parent protocol. The supplied window owns admission and durable history."""
import time
from browser_comparison_parent_protocol import (
    ProtocolError, receive_frame, send_frame, validate_done, validate_ready, validate_result)


def pair_complete(report):
    baseline, native = report['baseline'], report['native']
    return bool(report['failure'] is None and report['account_closed']
                and report['browser_cleanup_confirmed'] and report['native_nonempty_text']
                and baseline is not None and baseline['accepted']
                and baseline['status'] == 200 and baseline['response_complete']
                and baseline['complete_json'] and baseline['nonempty_text']
                and baseline['fetch_settled'] and native is not None
                and native['status'] == 200 and not native['failed']
                and native['complete_json'] and native['local_cleanup']
                and native['attempts'] == native['authorization_calls']
                == native['transport_starts'] == 1)


def run_once(plan, child, window, *, timeout=110, clock=time.monotonic):
    """admit/session/record/finish are synchronous, fail-closed owner operations.

    finish receives uncertainty even if a protocol operation fails. It must
    retain exclusion unless it durably records history and confirms cleanup.
    This module never releases that exclusion or changes a running service.
    """
    facts = dict(accepted=False, phase='admission', admitted=False, start_possible=False,
                 recorded=False, report=None, done=None, cleanup=None, failure=None)
    deadline = clock() + timeout
    if type(timeout) not in (int, float) or not 0 < timeout <= 110:
        raise ValueError('invalid comparison deadline')
    try:
        window.admit()
        facts['admitted'] = True
        facts['phase'] = 'launch'
        sock = child.launch()
        sock.settimeout(min(5, max(0.001, deadline - clock())))
        send_frame(sock, plan)
        validate_ready(receive_frame(sock), plan['nonce'])
        facts['phase'] = 'start'
        session = window.session()
        try:
            # A partial start write still requires conservative history.
            facts['start_possible'] = True
            send_frame(sock, dict(command='start', nonce=plan['nonce'], session_cookie=session))
        finally:
            session = None
        remaining = deadline - clock()
        if remaining <= 0:
            raise TimeoutError()
        sock.settimeout(remaining)
        facts['phase'] = 'result'
        report = validate_result(receive_frame(sock), plan['nonce'])
        facts['report'] = report
        window.record(report)
        facts['recorded'] = True
        sock.settimeout(min(5, max(0.001, deadline - clock())))
        send_frame(sock, dict(command='recorded', nonce=plan['nonce']))
        done = validate_done(receive_frame(sock), plan['nonce'])
        facts['done'] = done
        if done['local_pair_success'] != pair_complete(report) or done['failure'] != report['failure']:
            raise ProtocolError()
        facts['phase'] = 'cleanup'
    except BaseException:
        facts['failure'] = 'attempt_incomplete'
    finally:
        try:
            facts['cleanup'] = child.close()
        except BaseException:
            facts['failure'] = 'cleanup_unconfirmed'
        cleanup = facts['cleanup']
        if cleanup is not None and facts['failure'] is None:
            usage = cleanup['resources']
            clean = (cleanup['group_absent'] and not cleanup['forced'] and usage is not None
                     and usage['memory.swap.current'] == 0
                     and not any(usage['memory.events'].values())
                     and not any(usage['pids.events'].values()))
            facts['accepted'] = bool(clean and cleanup['exit_code'] == 0
                                     and facts['done'] is not None
                                     and facts['done']['local_pair_success'])
        if facts['admitted']:
            try:
                if window.finish(facts) is not True:
                    raise RuntimeError()
            except BaseException:
                facts['accepted'] = False
                facts['failure'] = 'history_publication_failed'
        if facts['failure'] is None:
            facts['phase'] = 'complete' if facts['accepted'] else 'refused'
    return facts
