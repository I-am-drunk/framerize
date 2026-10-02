#!/usr/bin/env bash
# framerize — Linux install. Idempotent; safe to re-run.
set -euo pipefail
cd "$(dirname "$0")/../.."   # repo root
ROOT="$(pwd)"
STATE_DIR="${FRAMERIZE_HOME:-$HOME/.framerize}"

echo "==> system deps"
if command -v apt-get >/dev/null; then
  sudo apt-get update -qq
  sudo apt-get install -y -qq build-essential pkg-config libssl-dev curl unzip \
    libdbus-1-3 libatk1.0-0 libatk-bridge2.0-0 libatspi2.0-0 libxcomposite1 \
    libxdamage1 libxfixes3 libxrandr2 libasound2 libgbm1 libxkbcommon0 \
    libnss3 libnspr4 libx11-6 libxcb1 libxext6 libexpat1
fi

echo "==> rust"
if ! command -v cargo >/dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
fi
source "$HOME/.cargo/env" 2>/dev/null || true

echo "==> chrome-headless-shell (the Castle engine)"
CHS_VER="141.0.7390.65"
CHS_DIR="$STATE_DIR/chrome-headless-shell-linux64"
if [ ! -x "$CHS_DIR/chrome-headless-shell" ]; then
  mkdir -p "$STATE_DIR"
  curl -sL -o /tmp/chs.zip "https://storage.googleapis.com/chrome-for-testing-public/$CHS_VER/linux64/chrome-headless-shell-linux64.zip"
  unzip -q /tmp/chs.zip -d "$STATE_DIR" && rm /tmp/chs.zip
fi

echo "==> build (release)"
cargo build --release -p framerize

mkdir -p "$STATE_DIR/profiles"

echo
echo "Done. Next:"
echo "  $ROOT/scripts/linux/start-daemon.sh"
echo "  $ROOT/scripts/linux/add-account.sh you@example.com"
echo "  $ROOT/scripts/fclaude          # Claude Code on the free route"
