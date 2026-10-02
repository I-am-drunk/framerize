#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
STATE_DIR="${FRAMERIZE_HOME:-$HOME/.framerize}"
mkdir -p "$STATE_DIR/profiles"
exec ./target/release/framerize daemon \
  --listen "${FRAMERIZE_LISTEN:-127.0.0.1:8787}" \
  --chrome "$STATE_DIR/chrome-headless-shell-linux64/chrome-headless-shell" \
  --profiles "$STATE_DIR/profiles" \
  --db "$STATE_DIR/framerize.db"
