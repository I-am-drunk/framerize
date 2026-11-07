"""Classify Framer edge security refusals before cooling a browser device."""
import json


_UPSTREAM_MARKERS = (
    '"request_id"', 'gen-', 'provider', 'moderation', 'no endpoints found',
    'no allowed providers', 'error_type', 'flagged', 'permission_error',
    'authentication_error', 'not_found_error', 'model', 'region', 'country',
    'permission', 'access denied', 'not authorized', 'not allowed',
)
_CASTLE_MARKERS = ("castle", "challenge", "captcha")
_SECURITY_MESSAGE = "this request was blocked by our security checks"


def is_castle_refusal(body):
    """Match the Rust 403 policy; provider refusals must not cool a device."""
    if isinstance(body, bytes):
        body = body.decode("utf-8", errors="replace")
    if not isinstance(body, str):
        return False
    lower = body.lower()
    if any(marker in lower for marker in _UPSTREAM_MARKERS):
        return False
    try:
        error = json.loads(body).get("error", {})
    except (ValueError, AttributeError):
        error = {}
    if not isinstance(error, dict):
        error = {}
    data = error.get("data") if isinstance(error, dict) else None
    if (error.get("status") == 403
            and error.get("message") == "This request was blocked by our security checks"
            and isinstance(data, dict) and data.get("reason") == "request_blocked"
            and isinstance(error.get("ref"), str) and error["ref"]
            and isinstance(error.get("timestamp"), str) and error["timestamp"]):
        return True
    message = error.get("message", body)
    return (isinstance(message, str) and message.strip().lower() == "forbidden") \
        or any(marker in lower for marker in _CASTLE_MARKERS)
