"""Structured host events."""
import json, os, sys, time

LOG_DIR = os.environ.get("FRAMERIZE_LOG_DIR")


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
    except Exception:
        pass
