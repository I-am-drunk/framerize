#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/../.."
STATE_DIR="${FRAMERIZE_HOME:-$HOME/.framerize}"
mkdir -p "$STATE_DIR/profiles"
exec ./target/release/framerize daemon \
  --listen "${FRAMERIZE_LISTEN:-127.0.0.1:8787}" \
  --chrome "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome" \
  --profiles "$STATE_DIR/profiles" \
  --db "$STATE_DIR/framerize.db"
