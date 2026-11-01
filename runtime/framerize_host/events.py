"""Structured host events."""
import json, os, re, sys, time

LOG_DIR = os.environ.get("FRAMERIZE_LOG_DIR")
_last_log_failure = None


def error_message(error):
    """Keep the failure summary, without browser call logs or credentials."""
    message = str(error).splitlines()[0] if str(error) else type(error).__name__
    message = re.sub(r'(?i)bearer\s+\S+', 'Bearer <redacted>', message)
    message = re.sub(r'https?://\S+', '<url>', message)
    message = re.sub(r'''(?i)\b(token|cookie|password|authorization|accessToken|cookie_header|session)["']?\s*[=:]\s*.*''',
                     r'\1=<redacted>', message)
    message = re.sub(r'\b[A-Za-z0-9_+/=-]{64,}\b', '<redacted>', message)
    return message[:600]


def error_cause(error):
    return f'{type(error).__name__}: {error_message(error)}'


def trace(msg):
    sys.stderr.write(f"[camoufox {time.strftime('%H:%M:%S')}] {msg}\n")
    sys.stderr.flush()


def emit(event, level="info", **fields):
    """Structured event — same NDJSON schema as the daemon (docs/logging.md).
    One line per event at $FRAMERIZE_LOG_DIR/events-<date>.ndjson."""
    if not LOG_DIR:
        return
    try:
        os.makedirs(LOG_DIR, exist_ok=True)
        rec = {"ts": time.strftime("%Y-%m-%dT%H:%M:%S.000Z", time.gmtime()),
               "level": level, "event": event, "source": "sidecar"}
        rec.update(fields)
        day = time.strftime("%Y-%m-%d", time.gmtime())
        with open(os.path.join(LOG_DIR, f"events-{day}.ndjson"), "a") as f:
            f.write(json.dumps(rec) + "\n")
    except Exception as error:
        global _last_log_failure
        now = time.monotonic()
        if _last_log_failure is None or now - _last_log_failure >= 60:
            _last_log_failure = now
            try:
                sys.stderr.write("Framerize event log write failed: " + error_cause(error) + "\n")
                sys.stderr.flush()
            except Exception:
                pass
