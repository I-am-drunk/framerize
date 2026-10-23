"""Retain only fixed native error codes, never provider messages or payloads."""
import json

NATIVE_CODES = frozenset({
    'InvalidRequest', 'MissingContext', 'UnsupportedStreaming',
    'MissingAuthorization', 'InvalidScope', 'ScopeClosed', 'Deadline', 'Cancelled',
    'Overloaded', 'Authorization', 'Transport', 'Body', 'BodyTooLarge', 'Upstream',
    'TerminalFailure', 'UnsupportedResponse', 'Cleanup', 'Ineligible',
    'PaidDisabled', 'CreditFloor', 'Internal',
})


def response_error(response):
    if response.getheader('Content-Type', '').split(';')[0].strip().lower() != 'application/json':
        return None
    raw = response.read(8193)
    if len(raw) > 8192:
        return None
    try:
        value = json.loads(raw)
        error = value.get('error') if isinstance(value, dict) else None
        code = error.get('code') if isinstance(error, dict) and error.get('type') == 'native_dispatch_error' else None
        return code if isinstance(code, str) and code in NATIVE_CODES else None
    except (ValueError, UnicodeError):
        return None
