#!/usr/bin/env python3
"""3-hour soak monitor for the framerize daemon (2026-10-06).

Probes the LIVE path (daemon on 127.0.0.1:8787) on a schedule and records
everything to ~/.framerize/soak-log.ndjson:
  - every 5 min: external time anchor, daemon health, tiny free probe
  - every 15 min: ultrafast STREAM probe (SSE timing + tier verification +
    ledger delta), codex rollout disconnect scan, relay/sidecar journal scan
  - every cycle: ledger tracking (any decrease = unexpected metering), chain
    error counts, dropped-frame lines, restarts

External time: worldtimeapi.org (fallback time.google.com) so elapsed time is
anchored independently of the box's clock.
"""
import asyncio, glob, json, os, re, time, urllib.request, uuid
from datetime import datetime, timezone

LOG = os.path.expanduser("~/.framerize/soak-log.ndjson")
DAEMON = "http://127.0.0.1:8787"
DURATION_S = 3 * 3600
TINY_EVERY_S = 300
DEEP_EVERY_S = 900
PROBE_MODEL = "google/gemini-2.5-flash"


def now_iso():
    return datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z")


def emit(rec):
    rec["ts"] = now_iso()
    line = json.dumps(rec)
    print(line, flush=True)
    with open(LOG, "a") as f:
        f.write(line + "\n")


def external_time():
    """UTC epoch from an external source (independent clock). Any HTTPS
    response carries a Date header — parse it from a reliable host."""
    import email.utils
    for url in ("https://www.google.com", "https://cloudflare.com", "https://github.com"):
        try:
            req = urllib.request.Request(url, method="HEAD")
            with urllib.request.urlopen(req, timeout=10) as r:
                dt = email.utils.parsedate_to_datetime(r.headers["Date"])
                return int(dt.timestamp())
        except Exception:
            continue
    return None


def http_json(method, path, body=None, timeout=30, headers=None, raw=False):
    req = urllib.request.Request(DAEMON + path, method=method)
    req.add_header("content-type", "application/json")
    for k, v in (headers or {}).items():
        req.add_header(k, v)
    data = json.dumps(body).encode() if body is not None else None
    t0 = time.time()
    with urllib.request.urlopen(req, data=data, timeout=timeout) as r:
        payload = r.read()
        return r.status, time.time() - t0, (payload if raw else json.loads(payload.decode()))


def ledger():
    try:
        _, _, accs = http_json("GET", "/api/accounts", timeout=10)
        return accs[0]["credit"]["available"]
    except Exception:
        return None


def tiny_probe():
    try:
        s, dt, j = http_json("POST", "/openai/v1/responses", {
            "model": PROBE_MODEL, "input": "say ok", "max_output_tokens": 10, "stream": False,
        }, timeout=60)
        text = ""
        try:
            text = j["output"][-1]["content"][0]["text"][:20]
        except Exception:
            pass
        return {"status": s, "ms": round(dt * 1000), "text": text}
    except Exception as e:
        return {"status": -1, "error": f"{type(e).__name__}: {e}"}


def ultrafast_stream_probe():
    """stream:true + service_tier:ultrafast through the daemon — verify the
    SSE flows, first byte is fast, the final object reports tier=ultrafast."""
    before = ledger()
    req = urllib.request.Request(DAEMON + "/openai/v1/responses", method="POST")
    req.add_header("content-type", "application/json")
    body = json.dumps({
        "model": "gpt-6-astra", "service_tier": "ultrafast",
        "input": "Reply with the single word: fast",
        "max_output_tokens": 300, "stream": True,
    }).encode()
    t0 = time.time()
    ttfb = None
    events = {}
    final = None
    try:
        with urllib.request.urlopen(req, data=body, timeout=120) as r:
            buf = b""
            while True:
                chunk = r.read(4096)
                if not chunk:
                    break
                if ttfb is None:
                    ttfb = time.time() - t0
                buf += chunk
                while b"\n\n" in buf:
                    frame, buf = buf.split(b"\n\n", 1)
                    for line in frame.decode(errors="replace").splitlines():
                        if line.startswith("event: "):
                            ev = line[7:]
                            events[ev] = events.get(ev, 0) + 1
                        elif line.startswith("data: ") and '"response.completed"' in line:
                            try:
                                final = json.loads(line[6:]).get("response")
                            except Exception:
                                pass
    except Exception as e:
        return {"error": f"{type(e).__name__}: {e}", "ledger_before": before}
    total = time.time() - t0
    after = ledger()
    tier = None
    if final:
        tier = final.get("service_tier") or (final.get("usage") or {}).get("service_tier")
    return {
        "ttfb_s": round(ttfb or -1, 2),
        "total_s": round(total, 2),
        "events": events,
        "tier": tier,
        "ledger_before": before,
        "ledger_after": after,
        "unmetered": (before is not None and after is not None and abs(after - before) < 1e-9),
    }


def journal_counts(since_iso):
    """Count key lines in the daemon journal since a timestamp."""
    import subprocess
    out = subprocess.run(
        ["journalctl", "--user", "-u", "framerize.service", "--no-pager", "--since", since_iso],
        capture_output=True, text=True, timeout=30).stdout
    return {
        "chains_ok": len(re.findall(r"ai\.chain.*outcome..ok", out)),
        "err_403": out.count(" 403 "),
        "err_429": out.count(" 429 "),
        "dropped_frames": out.count("event channel full"),
        "panics": out.count("panicked at"),
        "restarts": out.count("framerize daemon on 127.0.0.1:8787"),
        "chain_errors": len(re.findall(r"\[nativechain\].*(died|error|failed)", out)) + len(re.findall(r"\[freechat\].*(died|error|failed)", out)),
    }


def codex_disconnect_scan(minutes):
    files = glob.glob(os.path.expanduser("~/.codex/sessions/2026/*/*/*.jsonl"))
    recent = [f for f in files if os.path.getmtime(f) > time.time() - minutes * 60]
    hits = 0
    for f in recent:
        try:
            hits += open(f, errors="replace").read().count("responseStreamDisconnected")
        except Exception:
            pass
    return {"rollouts_touched": len(recent), "stream_disconnects": hits}


async def main():
    ext0 = external_time()
    mono0 = time.monotonic()
    emit({"kind": "soak-start", "external_epoch": ext0, "mono": mono0,
          "note": "3h soak: tiny probe /5min, ultrafast-stream deep probe /15min, journal+rollout scans"})
    last_tiny = last_deep = 0.0
    last_journal_ts = now_iso()
    cycle = 0
    while time.monotonic() - mono0 < DURATION_S:
        now = time.monotonic()
        if now - last_tiny >= TINY_EVERY_S:
            last_tiny = now
            cycle += 1
            health = None
            try:
                s, dt, _ = http_json("GET", "/api/health", timeout=10, raw=True)
                health = {"status": s, "ms": round(dt * 1000)}
            except Exception as e:
                health = {"status": -1, "error": str(e)}
            emit({"kind": "cycle", "n": cycle, "elapsed_s": round(now - mono0),
                  "external_epoch": external_time(),
                  "health": health, "tiny_probe": tiny_probe(), "ledger": ledger()})
        if now - last_deep >= DEEP_EVERY_S:
            last_deep = now
            jc = journal_counts(last_journal_ts)
            last_journal_ts = now_iso()
            emit({"kind": "deep", "elapsed_s": round(now - mono0),
                  "ultrafast_stream": ultrafast_stream_probe(),
                  "journal_window": jc,
                  "codex": codex_disconnect_scan(20)})
        await asyncio.sleep(15)
    emit({"kind": "soak-end", "external_epoch": external_time(),
          "mono_elapsed_s": round(time.monotonic() - mono0), "ledger": ledger()})


if __name__ == "__main__":
    asyncio.run(main())
