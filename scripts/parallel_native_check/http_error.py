"""Retain only fixed native error codes, never provider messages or payloads."""
import http.client
import json

NATIVE_CODES = frozenset({
    'InvalidRequest', 'MissingContext', 'UnsupportedStreaming',
    'MissingAuthorization', 'InvalidScope', 'ScopeClosed', 'Deadline', 'Cancelled',
    'Overloaded', 'Authorization', 'Transport', 'Body', 'BodyTooLarge', 'Upstream',
    'TerminalFailure', 'UnsupportedResponse', 'Cleanup', 'Ineligible',
    'PaidDisabled', 'CreditFloor', 'Internal',
})


def security_envelope(value, status):
    error = value.get('error') if isinstance(value, dict) else None
    if status != 403 or not isinstance(error, dict):
        return False
    data = error.get('data')
    return (type(error.get('status')) is int and error['status'] == 403
            and error.get('message') == 'This request was blocked by our security checks'
            and isinstance(data, dict) and data.get('reason') == 'request_blocked'
            and isinstance(error.get('ref'), str) and bool(error['ref'])
            and isinstance(error.get('timestamp'), str) and bool(error['timestamp']))


def response_details(response):
    media = response.getheader('Content-Type', '').split(';')[0].strip().lower()
    kind = {'application/json': 'json', 'text/html': 'html', 'text/event-stream': 'event_stream'}.get(media, 'other')
    details = {'native_error_code': None, 'content_kind': kind, 'json_valid': False,
               'json_bytes': None, 'json_truncated': False, 'framer_security_envelope': False,
               'body_framing': 'unobserved'}
    if media != 'application/json':
        return details
    try:
        raw = response.read(8193)
    except http.client.IncompleteRead as error:
        raw = error.partial
        details['body_framing'] = 'incomplete'
    else:
        if response.isclosed() and response.length in (None, 0):
            details['body_framing'] = 'complete'
        elif len(raw) < 8193:
            # A short blocking HTTPResponse read reached EOF. Length-framed
            # bodies still owe bytes; close-delimited bodies end at that EOF.
            if response.length is not None:
                details['body_framing'] = 'incomplete'
            elif not response.chunked:
                details['body_framing'] = 'complete'
    details['json_bytes'] = len(raw)
    if len(raw) > 8192:
        details['json_truncated'] = True
        return details
    try:
        value = json.loads(raw)
        details['json_valid'] = True
        error = value.get('error') if isinstance(value, dict) else None
        code = error.get('code') if isinstance(error, dict) and error.get('type') == 'native_dispatch_error' else None
        if isinstance(code, str) and code in NATIVE_CODES:
            details['native_error_code'] = code
        details['framer_security_envelope'] = security_envelope(value, getattr(response, 'status', None))
    except (ValueError, UnicodeError):
        pass
    return details


def response_error(response):
    return response_details(response)['native_error_code']
