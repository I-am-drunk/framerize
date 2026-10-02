#!/usr/bin/env bash
# framerize bootstrap — ONE command, run once, on any state of mess.
# Reuses an existing build (never rebuilds when a binary exists), cleans up
# nested accidental clones, fetches the Castle engine, starts the daemon,
# adds your Framer account, and prints the fcd command.
set -euo pipefail

say() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }
STATE="$HOME/.framerize"
BIN_DEST="$HOME/.local/bin/framerize"
CHS="$STATE/chrome-headless-shell-linux64/chrome-headless-shell"
BASE="http://127.0.0.1:8787"

# ---------- 1. find the newest existing build; only build if NONE ----------
say "looking for an existing framerize binary"
BIN=""
for c in "$HOME"/framerize/target/release/framerize "$HOME"/framerize/*/target/release/framerize "$HOME"/framerize/*/*/target/release/framerize; do
  [ -x "$c" ] && BIN="$c" && break   # first match = outermost clone
done
if [ -z "$BIN" ]; then
  say "no binary found — building (one time, ~6 min)"
  # ensure we have a repo to build from
  if [ ! -d "$HOME/framerize/.git" ]; then
    git clone https://github.com/I-am-drunk/framerize.git "$HOME/framerize"
  fi
  cd "$HOME/framerize"
  command -v cargo >/dev/null || { curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable; source "$HOME/.cargo/env"; }
  command -v cc >/dev/null || { sudo apt-get update -qq && sudo apt-get install -y -qq build-essential pkg-config libssl-dev; }
  cargo build --release -p framerize
  BIN="$HOME/framerize/target/release/framerize"
fi
echo "using binary: $BIN"
mkdir -p "$HOME/.local/bin"
cp "$BIN" "$BIN_DEST"
echo "installed to $BIN_DEST"
install -m755 "$ROOT/scripts/fcd" "$HOME/.local/bin/fcd"
echo "installed fcd to $HOME/.local/bin/fcd"

# ---------- 2. clean up the nested-clone mess ----------
say "cleaning up nested clones"
# Keep the clone that owns the binary if it's a repo; otherwise keep ~/framerize.
KEEP="$(dirname "$(dirname "$(dirname "$BIN")")")"
echo "keeping: $KEEP"
for d in "$HOME"/framerize; do
  [ -d "$d" ] || continue
  find "$d" -mindepth 1 -maxdepth 1 -type d -name framerize | while read -r nest; do
    echo "removing nested clone: $nest"
    rm -rf "$nest"
  done
done
# make sure ~/.local/bin is on PATH for later
case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) echo 'export PATH="$HOME/.local/bin:$PATH"' >> "$HOME/.bashrc";; esac

# ---------- 3. Castle engine ----------
# Camoufox (stealth Firefox) is the recommended engine — it's the config that
# passed Castle in production. It needs ~2.5GB disk (663MB download + extract).
# Skip with --engine full-chrome to use installed Chrome instead.
ENGINE="${FRAMERIZE_ENGINE:-camoufox}"
for arg in "$@"; do case "$arg" in --engine=*) ENGINE="${arg#--engine=}";; esac; done
if [ "$ENGINE" = "camoufox" ]; then
  if [ -x "$HOME/.framerize/venv/bin/python3" ] && ls "$HOME/.framerize"/venv/lib/python*/site-packages/camoufox >/dev/null 2>&1; then
    say "camoufox engine already installed"
  else
    bash "$ROOT/scripts/linux/install-camoufox.sh"
  fi
fi
if false; then
# ---------- legacy: chrome-headless-shell (known-bad Castle fingerprint) ----------
if [ -x "$CHS" ]; then
  say "castle engine already present"
else
  say "downloading castle engine (chrome-headless-shell, ~100MB)"
  mkdir -p "$STATE"
  curl -sL -o /tmp/chs.zip "https://storage.googleapis.com/chrome-for-testing-public/141.0.7390.65/linux64/chrome-headless-shell-linux64.zip"
  unzip -q /tmp/chs.zip -d "$STATE" && rm /tmp/chs.zip
fi
# its shared libraries (idempotent)
if command -v apt-get >/dev/null; then
  sudo apt-get install -y -qq libdbus-1-3 libatk1.0-0 libatk-bridge2.0-0 libatspi2.0-0 \
    libxcomposite1 libxdamage1 libxfixes3 libxrandr2 libasound2 libgbm1 libxkbcommon0 \
    libnss3 libnspr4 libx11-6 libxcb1 libxext6 libexpat1 >/dev/null 2>&1 || true
fi

fi

# ---------- 4. (re)start the daemon ----------
say "starting the daemon"
pkill -x framerize 2>/dev/null || true        # stop any running daemon
pkill -x chrome-headless 2>/dev/null || true  # and orphaned engines
sleep 1
mkdir -p "$STATE/profiles"
if [ "$ENGINE" = "camoufox" ]; then
  ENGINE_ARGS=(--engine camoufox)
else
  CHROME_BIN="$(command -v google-chrome || echo "$CHS")"
  ENGINE_ARGS=(--engine "${ENGINE:-full-chrome}" --chrome "$CHROME_BIN")
fi
nohup "$BIN_DEST" daemon --listen 127.0.0.1:8787 "${ENGINE_ARGS[@]}" \
  --profiles "$STATE/profiles" --db "$STATE/framerize.db" \
  > "$STATE/daemon.log" 2>&1 &
for i in $(seq 1 20); do
  curl -s -m 2 "$BASE/api/health" >/dev/null 2>&1 && break
  sleep 0.5
done
if curl -s -m 2 "$BASE/api/health" >/dev/null 2>&1; then
  echo "daemon up at $BASE (log: $STATE/daemon.log)"
else
  echo "DAEMON FAILED — last log lines:"; tail -5 "$STATE/daemon.log"; exit 1
fi

# ---------- 5. add the Framer account ----------
say "add your Framer account"
EMAIL="${1:-}"
[ -z "$EMAIL" ] && read -rp "Framer email: " EMAIL
ACCT=$(cat /proc/sys/kernel/random/uuid 2>/dev/null || uuidgen)
echo "account id: $ACCT"
curl -s -X POST "$BASE/api/accounts/begin" -H 'content-type: application/json' \
  -d "{\"account_id\":\"$ACCT\",\"email\":\"$EMAIL\"}"
echo
echo ">> magic link sent to $EMAIL — open the email, copy the link, take the #token= UUID"
read -rp "paste the token UUID here: " TOKEN
curl -s -X POST "$BASE/api/accounts/complete" -H 'content-type: application/json' \
  -d "{\"account_id\":\"$ACCT\",\"email_token\":\"$TOKEN\",\"alias\":\"$EMAIL\",\"project_id\":\"UtVRwMnJTYSkR8C46sQM\"}"
echo

# ---------- 6. Claude Code ----------
say "Claude Code"
if ! command -v claude >/dev/null; then
  echo "installing (needs node/npm)…"
  if ! command -v npm >/dev/null; then sudo apt-get install -y -qq nodejs npm; fi
  npm i -g @anthropic-ai/claude-code
fi
cat <<MSG

All set. From now on:

  fcd                # Claude Code on the FREE route (unmetered, never-fail chain)
                   #   (or: ANTHROPIC_BASE_URL=$BASE/anthropic ANTHROPIC_AUTH_TOKEN=local claude)

Paid/metered passthrough if ever needed:
  ANTHROPIC_BASE_URL=$BASE/paid/anthropic ANTHROPIC_AUTH_TOKEN=local claude
MSG
