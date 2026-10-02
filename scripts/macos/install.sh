#!/usr/bin/env bash
# framerize — macOS install. Uses the installed Chrome as the Castle engine
# (native WKWebView host is the planned successor — see
# reference/analysis/browser-engine-comparison.md).
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT="$(pwd)"
STATE_DIR="${FRAMERIZE_HOME:-$HOME/.framerize}"

command -v brew >/dev/null || { echo "install Homebrew first: https://brew.sh"; exit 1; }
command -v cargo >/dev/null || curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
source "$HOME/.cargo/env" 2>/dev/null || true
command -v openssl >/dev/null || brew install openssl@3

CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
[ -x "$CHROME" ] || { echo "install Google Chrome first"; exit 1; }

cargo build --release -p framerize
mkdir -p "$STATE_DIR/profiles"

echo
echo "Done. Next:"
echo "  $ROOT/scripts/macos/start-daemon.sh"
echo "  $ROOT/scripts/macos/add-account.sh you@example.com"
echo "  $ROOT/scripts/fc"
