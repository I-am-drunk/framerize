"""Bounded wire helpers for the comparison child at 6c8ab52.

The caller owns deadlines, message order, durable recording and final acceptance.
These helpers never create sockets or apply cooldowns. Validation is not release.
"""
from functools import wraps
import json
import math
import struct

MAX_SEND_BYTES = 96 * 1024
MAX_RECEIVE_BYTES = 8 * 1024
_BODY_LIMIT = 64 * 1024
_U64_MAX = (1 << 64) - 1
_FAILURES = frozenset(("MissingAuthority", "Cooling", "WavePaused", "Admission",
    "Context", "Session", "Bootstrap", "Source", "Dispatch", "Cleanup",
    "CooldownPublication", "OwnerLost"))
_CONTENT_TYPES = frozenset(("Json", "EventStream", "Other", "Missing"))
_POLICIES = frozenset(("Castle", "Upstream"))
_BASES = frozenset(("ProviderMarker", "FramerSecurityEnvelope", "FramerMarker",
                    "ElapsedFallback"))


class ProtocolError(ValueError):
    """A fixed public error, with no payload or underlying exception attached."""

    def __init__(self):
        super().__init__("invalid browser comparison protocol")


def _safe(function):
    @wraps(function)
    def checked(*args, **kwargs):
        try:
            return function(*args, **kwargs)
        except Exception:
            pass
        try:
            raise ProtocolError() from None
        except ProtocolError as error:
            error.__context__ = None
            raise
    return checked


def _require(condition):
    if not condition:
        raise ProtocolError()


def _json_types(value):
    kind = type(value)
    if value is None or kind in (bool, int):
        return
    if kind is str:
        value.encode("utf-8", errors="strict")
    elif kind is float:
        _require(math.isfinite(value))
    elif kind is list:
        for item in value:
            _json_types(item)
    elif kind is dict:
        for key, item in value.items():
            _require(type(key) is str)
            _json_types(key)
            _json_types(item)
    else:
        raise ProtocolError()


def _unique_object(pairs):
    result = {}
    for key, value in pairs:
        _require(key not in result)
        result[key] = value
    return result


def _nonfinite(_):
    raise ProtocolError()


def _read_exact(sock, size):
    data = bytearray()
    while len(data) < size:
        part = sock.recv(size - len(data))
        _require(type(part) is bytes and 0 < len(part) <= size - len(data))
        data.extend(part)
    return bytes(data)


@_safe
def send_frame(sock, value):
    """Send one object; caller sets the deadline and must not retry partial I/O."""
    _require(type(value) is dict)
    _json_types(value)
    body = json.dumps(value, ensure_ascii=False, allow_nan=False,
                      separators=(",", ":")).encode("utf-8")
    _require(0 < len(body) <= MAX_SEND_BYTES)
    sock.sendall(struct.pack("!I", len(body)) + body)


@_safe
def receive_frame(sock):
    """Read one object, leaving any following frame for the next call."""
    size = struct.unpack("!I", _read_exact(sock, 4))[0]
    _require(0 < size <= MAX_RECEIVE_BYTES)
    body = _read_exact(sock, size)
    value = json.loads(body.decode("utf-8", errors="strict"),
                       object_pairs_hook=_unique_object, parse_constant=_nonfinite)
    _require(type(value) is dict)
    _json_types(value)
    return value


def _keys(value, names):
    _require(type(value) is dict)
    _require(all(type(key) is str for key in value) and set(value) == set(names.split()))


def _number(value, maximum, *, minimum=0, nullable=False):
    _require((nullable and value is None)
             or (type(value) is int and minimum <= value <= maximum))


def _label(value, labels, *, nullable=False):
    _require((nullable and value is None) or (type(value) is str and value in labels))


def _bools(value, names):
    _require(all(type(value[name]) is bool for name in names.split()))


def _message(value, nonce, command, names):
    _keys(value, names)
    _require(type(nonce) is str and len(nonce) == 32
             and all(char in "0123456789abcdefABCDEF" for char in nonce))
    _require(type(value["nonce"]) is str and value["nonce"] == nonce)
    _require(type(value["command"]) is str and value["command"] == command)


def _refusal(value):
    if value is None:
        return None
    _keys(value, "policy basis")
    _label(value["policy"], _POLICIES)
    _label(value["basis"], _BASES)
    return dict(value)


def _baseline(value):
    if value is None:
        return None
    _keys(value, "accepted status content_type response_complete complete_json "
          "nonempty_text body_bytes transport_starts transport_finishes fetch_settled refusal")
    _bools(value, "accepted response_complete complete_json nonempty_text fetch_settled")
    _number(value["status"], 599, minimum=100, nullable=True)
    _label(value["content_type"], _CONTENT_TYPES)
    _number(value["body_bytes"], _BODY_LIMIT)
    for key in ("transport_starts", "transport_finishes"):
        _number(value[key], 1, nullable=True)
    refusal = _refusal(value["refusal"])
    # The host's BaselineReport::parse already requires these for acceptance.
    if value["accepted"]:
        _require(value["status"] == 200 and value["content_type"] == "Json"
                 and value["response_complete"] and value["complete_json"]
                 and value["nonempty_text"] and value["fetch_settled"]
                 and value["transport_starts"] == 1 and value["transport_finishes"] == 1
                 and value["body_bytes"] > 0 and refusal is None)
    return {**value, "refusal": refusal}


@_safe
def validate_ready(value, nonce):
    """Check the exact ready envelope. Returns None; grants no admission."""
    _message(value, nonce, "ready", "command nonce")


def _native(value):
    if value is None:
        return None
    _keys(value, "complete_json status attempts authorization_calls transport_starts "
          "local_cleanup failed refusal")
    _bools(value, "complete_json local_cleanup failed")
    # LocalReport has Option<u16> and usize counts, not the baseline's bounds.
    _number(value["status"], 65535, nullable=True)
    for key in ("attempts", "authorization_calls", "transport_starts"):
        _number(value[key], _U64_MAX)
    return {**value, "refusal": _refusal(value["refusal"])}


@_safe
def validate_result(value, nonce):
    """Return a detached, typed report, including failed or uncertain facts.

    This performs no cross-message or local-success decision. See the source note.
    """
    _message(value, nonce, "result", "command nonce report")
    report = value["report"]
    _keys(report, "baseline native native_nonempty_text browser_cleanup_confirmed "
          "account_closed failure")
    _bools(report, "native_nonempty_text browser_cleanup_confirmed account_closed")
    _label(report["failure"], _FAILURES, nullable=True)
    return {**report, "baseline": _baseline(report["baseline"]),
            "native": _native(report["native"])}


@_safe
def validate_done(value, nonce):
    """Check a done envelope, not completion of the caller's protocol sequence."""
    _message(value, nonce, "done", "command nonce local_pair_success failure")
    _bools(value, "local_pair_success")
    _label(value["failure"], _FAILURES, nullable=True)
    _require(not value["local_pair_success"] or value["failure"] is None)
    return dict(value)
