#!/usr/bin/env bash
# install-camoufox.sh — the stealth engine framerize's --engine camoufox needs.
# Installs into ~/.framerize/venv (the daemon finds it automatically).
set -euo pipefail

say() { echo "==> $*"; }

command -v python3 >/dev/null || { sudo apt-get update -qq && sudo apt-get install -y -qq python3 python3-venv; }
python3 -m venv ~/.framerize/venv 2>/dev/null || { sudo apt-get install -y -qq python3-venv && python3 -m venv ~/.framerize/venv; }

say "installing camoufox + playwright (~40MB pip, ~300MB browser fetch)"
~/.framerize/venv/bin/pip install -q --upgrade pip
~/.framerize/venv/bin/pip install -q camoufox playwright
~/.framerize/venv/bin/python -m camoufox fetch

say "done. Run the daemon with: framerize daemon --engine camoufox ..."
echo "  (or make it the default: export FRAMERIZE_ENGINE=camoufox)"
