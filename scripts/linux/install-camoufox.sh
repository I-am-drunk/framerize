#!/usr/bin/env bash
# install-camoufox.sh — the stealth engine framerize's --engine camoufox needs.
# Installs into ~/.framerize/venv (the daemon finds it automatically).
set -euo pipefail

say() { echo "==> $*"; }

command -v python3 >/dev/null || { sudo apt-get update -qq && sudo apt-get install -y -qq python3 python3-venv; }
python3 -m venv ~/.framerize/venv 2>/dev/null || { sudo apt-get install -y -qq python3-venv && python3 -m venv ~/.framerize/venv; }

AVAIL=$(df --output=avail -m ~ 2>/dev/null | tail -1 | tr -d ' ')
if [ -n "$AVAIL" ] && [ "$AVAIL" -lt 2600 ]; then
  echo "WARNING: ${AVAIL}MB free in ~ — camoufox needs ~2600MB peak (663MB download + extract)." >&2
  echo "  Free space first, or run the daemon with --engine full-chrome instead." >&2
fi

say "installing camoufox + playwright (~40MB pip, ~663MB browser fetch, ~2.5GB peak disk)"
~/.framerize/venv/bin/pip install -q --upgrade pip
~/.framerize/venv/bin/pip install -q camoufox playwright

# A previous ENOSPC leaves a broken partial extraction that poisons reruns.
if ! find ~/.cache/camoufox/browsers -name camoufox -type f 2>/dev/null | grep -q .; then
  rm -rf ~/.cache/camoufox/browsers/official/* 2>/dev/null || true
fi
~/.framerize/venv/bin/python -m camoufox fetch

# camoufox fetch exits 0 even when it fails — verify the binary actually landed.
if ! find ~/.cache/camoufox/browsers -name camoufox -type f 2>/dev/null | grep -q .; then
  echo "" >&2
  echo "ERROR: camoufox fetch failed (see above — usually disk space)." >&2
  echo "  Free ~1GB and re-run this script. Biggest wins:" >&2
  echo "    rm -rf ~/framerize/target ~/.framerize/chrome-headless-shell-linux64; sudo apt-get clean" >&2
  exit 1
fi

say "done. Run the daemon with: framerize daemon --engine camoufox ..."
echo "  (or make it the default: export FRAMERIZE_ENGINE=camoufox)"
