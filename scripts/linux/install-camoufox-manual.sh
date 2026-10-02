#!/usr/bin/env bash
# install-camoufox-manual.sh — install the camoufox browser from a PREBUILT
# cache tarball instead of `camoufox fetch` (whose downloader can't resume
# and stalls on slow GitHub routes). Use when install-camoufox.sh stalls.
#
#   bash scripts/linux/install-camoufox-manual.sh [URL]
#
# URL defaults to the framerize release asset; pass a mirror URL as $1.
# Cleans old/partial camoufox data first, downloads RESUMABLY, verifies.
set -euo pipefail
say() { echo "==> $*"; }

URL="${1:-${CFX_URL:-}}"
if [ -z "$URL" ]; then
  echo "usage: bash $0 <tarball-url>" >&2
  echo "  (get the current URL from the framerize release notes / your setup message)" >&2
  exit 1
fi

# python env must exist (pip packages); create if missing
if [ ! -x "$HOME/.framerize/venv/bin/python3" ]; then
  say "creating venv + installing camoufox/playwright python packages"
  bash "$(dirname "$0")/install-camoufox.sh" --no-fetch || {
    python3 -m venv ~/.framerize/venv
    ~/.framerize/venv/bin/pip install -q camoufox playwright
  }
fi

# ---- cleanup: every old/partial camoufox artifact (requested) ----
say "cleaning old camoufox data"
rm -rf ~/.cache/camoufox/browsers ~/.cache/camoufox/downloads /tmp/camoufox* /tmp/cfx 2>/dev/null || true
mkdir -p ~/.cache/camoufox

AVAIL=$(df --output=avail -m ~ | tail -1 | tr -d ' ')
say "free space: ${AVAIL}MB (need ~1600MB: tarball + extracted)"
[ "$AVAIL" -lt 1600 ] && { echo "not enough space — free some and retry" >&2; exit 1; }

# ---- resumable download (retries pick up where they stopped) ----
say "downloading camoufox cache tarball (~630MB, resumable)"
for attempt in 1 2 3 4 5 6 7 8 9 10 11 12; do
  curl -L -C - --retry 0 --connect-timeout 20 -o /tmp/camoufox-cache.tar.gz "$URL" && break
  say "attempt $attempt failed/stalled — resuming in 3s"
  sleep 3
done
ls -la /tmp/camoufox-cache.tar.gz

# ---- place into the camoufox cache ----
say "extracting into ~/.cache/camoufox"
tar -xzf /tmp/camoufox-cache.tar.gz -C ~/.cache/camoufox/
rm /tmp/camoufox-cache.tar.gz

# ---- runtime libraries Firefox needs (Chrome pulls most; make sure) ----
if command -v apt-get >/dev/null; then
  sudo apt-get install -y -qq libgtk-3-0 libx11-xcb1 libdbus-glib-1-2 libxt6 libasound2 libxcomposite1 libxdamage1 libxrandr2 libxkbcommon0 libpango-1.0-0 libcairo2 >/dev/null 2>&1 || true
fi

# ---- verify: binary + metadata + compat flag, and the launcher agrees ----
BIN="$HOME/.cache/camoufox/browsers/official/152.0.4-beta.31-3a7958c8/camoufox"
[ -x "$BIN" ] || { echo "ERROR: camoufox binary missing at $BIN" >&2; exit 1; }
[ -f "${BIN%/*}/version.json" ] || { echo "ERROR: version.json missing" >&2; exit 1; }
[ -f "$HOME/.cache/camoufox/.0.5_FLAG" ] || { echo "ERROR: .0.5_FLAG missing" >&2; exit 1; }
if ! "$HOME/.framerize/venv/bin/python" -c "from camoufox.pkgman import camoufox_path; print('launcher sees:', camoufox_path(download_if_missing=False))" 2>/dev/null; then
  echo "ERROR: camoufox python package does not recognize the install" >&2
  exit 1
fi

say "camoufox installed and recognized. Start the daemon:"
echo "  framerize daemon --engine camoufox --profiles ~/.framerize/profiles --db ~/.framerize/framerize.db"
