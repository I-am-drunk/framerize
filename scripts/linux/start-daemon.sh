#!/usr/bin/env bash
# start-daemon.sh — engine priority: camoufox (stealth, proven) > full-chrome.
set -euo pipefail
STATE_DIR="${FRAMERIZE_HOME:-$HOME/.framerize}"
mkdir -p "$STATE_DIR/profiles"

BIN="$(command -v framerize || true)"
[ -n "$BIN" ] || BIN="$(dirname "$0")/../../target/release/framerize"

ENGINE="${FRAMERIZE_ENGINE:-}"
if [ -z "$ENGINE" ]; then
  if [ -x "$STATE_DIR/venv/bin/python3" ] && [ -d "$STATE_DIR/venv/lib" ] && ls "$STATE_DIR"/venv/lib/python*/site-packages/camoufox >/dev/null 2>&1; then
    ENGINE=camoufox
  else
    ENGINE=full-chrome
  fi
fi

ARGS=(daemon
  --listen "${FRAMERIZE_LISTEN:-127.0.0.1:8787}"
  --engine "$ENGINE"
  --profiles "$STATE_DIR/profiles"
  --db "$STATE_DIR/framerize.db")

if [ "$ENGINE" != "camoufox" ]; then
  CHROME="$(command -v google-chrome || echo "$STATE_DIR/chrome-headless-shell-linux64/chrome-headless-shell")"
  ARGS+=(--chrome "$CHROME")
fi

echo "starting framerize daemon (engine: $ENGINE, log: terminal)"
exec "$BIN" "${ARGS[@]}"
