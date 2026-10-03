#!/usr/bin/env bash
# server-setup.sh — full framerize server build-out, idempotent.
# Run: bash scripts/linux/server-setup.sh
# Phases: system update → remove old docker/leftovers → build deps → rust →
# clone/pull framerize → release build → camoufox engine → daemon service →
# Claude Code (fcd) + Codex (fcx) launchers → verify.
set -uo pipefail
LOG=/var/log/framerize-setup.log
exec > >(tee -a "$LOG") 2>&1
say() { echo; echo "==> [$(date +%H:%M:%S)] $*"; }
ok()  { echo "    ✓ $*"; }
warn(){ echo "    ! $*"; }

STATE="$HOME/.framerize"
REPO="$HOME/framerize"

# ---------- 0. sanity ----------
say "0. environment"
. /etc/os-release 2>/dev/null || true
echo "    os: ${PRETTY_NAME:-unknown} · arch: $(uname -m) · kernel: $(uname -r)"
echo "    mem: $(free -h | awk '/Mem:/{print $2}') · disk free: $(df -h / | awk 'NR==2{print $4}')"
[ "$(uname -m)" = "x86_64" ] || { echo "ERROR: x86_64 only (camoufox build)"; exit 1; }

# ---------- 1. system update ----------
say "1. system update (apt update + upgrade, noninteractive)"
export DEBIAN_FRONTEND=noninteractive
sudo apt-get update -qq
sudo apt-get -y -qq -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold upgrade || warn "upgrade had errors (continuing)"
sudo apt-get -y -qq -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold dist-upgrade || warn "dist-upgrade had errors (continuing)"
ok "system updated"

# ---------- 2. remove old docker & leftover junk ----------
say "2. removing old docker + leftover components"
if command -v docker >/dev/null || dpkg -l | grep -qE "docker|containerd"; then
  sudo systemctl stop docker docker.socket containerd 2>/dev/null
  sudo apt-get -y -qq purge docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin docker.io docker-doc docker-compose podman-docker containerd runc 2>/dev/null
  sudo apt-get -y -qq autoremove --purge
  sudo rm -rf /var/lib/docker /var/lib/containerd /etc/docker
  ok "docker removed"
else
  ok "no docker found"
fi

# ---------- 3. build dependencies ----------
say "3. build + runtime dependencies"
sudo apt-get install -y -qq \
  git curl wget unzip jq ca-certificates \
  build-essential pkg-config libssl-dev \
  python3 python3-venv python3-pip \
  libdbus-1-3 libdbus-glib-1-2 libatk1.0-0 libatk-bridge2.0-0 libatspi2.0-0 \
  libxcomposite1 libxdamage1 libxfixes3 libxrandr2 libasound2 libgbm1 libxkbcommon0 \
  libnss3 libnspr4 libx11-6 libxcb1 libxext6 libexpat1 libgtk-3-0 libx11-xcb1 \
  libxt6 libpango-1.0-0 libcairo2 \
  nodejs npm 2>/dev/null || warn "some optional packages failed (continuing)"
ok "dependencies installed"

# ---------- 4. rust ----------
say "4. rust toolchain"
if ! command -v cargo >/dev/null; then
  curl -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable --profile minimal
fi
. "$HOME/.cargo/env"
ok "$(cargo --version)"

# ---------- 5. framerize repo ----------
say "5. framerize repo"
if [ -d "$REPO/.git" ]; then
  git -C "$REPO" pull --ff-only || warn "pull failed — using existing tree"
else
  git clone https://x-access-token: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@github.com/I-am-drunk/framerize.git "$REPO"
fi
ok "repo at $REPO ($(git -C "$REPO" log --oneline -1 | cut -c1-50))"

# ---------- 6. release build ----------
say "6. release build (this takes a while on a server)"
cd "$REPO"
cargo build --release -p framerize
install -m755 target/release/framerize "$HOME/.local/bin/framerize" 2>/dev/null || { mkdir -p "$HOME/.local/bin" && install -m755 target/release/framerize "$HOME/.local/bin/framerize"; }
install -m755 scripts/fcd "$HOME/.local/bin/fcd"
case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) echo 'export PATH="$HOME/.local/bin:$PATH"' >> "$HOME/.bashrc";; esac
ok "framerize + fcd installed to ~/.local/bin"

# ---------- 7. camoufox engine ----------
say "7. camoufox stealth engine"
bash "$REPO/scripts/linux/install-camoufox.sh"
cp scripts/camoufox_host.py "$STATE/camoufox_host.py"  # external override (hotfixes via git pull)
ok "camoufox ready"

# ---------- 8. Codex + fcx launcher ----------
say "8. codex + fcx launcher"
if ! command -v codex >/dev/null; then
  sudo npm install -g @openai/codex 2>/dev/null || npm install -g @openai/codex
fi
mkdir -p "$HOME/.codex"
cat > "$HOME/.codex/config.toml" <<'TOML'
# framerize provider: the daemon's free chain (bare /openai = free; /paid/openai = metered).
model = "openai/gpt-4.1"
model_provider = "framerize"

[model_providers.framerize]
name = "framerize (local Framer gateway)"
base_url = "http://127.0.0.1:8787/openai/v1"
wire_api = "responses"
env_key = "FRAMERIZE_LOCAL"
TOML
cat > "$HOME/.local/bin/fcx" <<'FCX'
#!/usr/bin/env bash
# fcx — Codex backed by framerize's free chain.
export FRAMERIZE_LOCAL="${FRAMERIZE_LOCAL:-framerize-local}"
exec codex "$@"
FCX
chmod +x "$HOME/.local/bin/fcx"
ok "codex configured (~/.codex/config.toml → framerize) + fcx installed"

# ---------- 9. Claude Code ----------
say "9. claude code (fcd)"
if ! command -v claude >/dev/null; then
  sudo npm install -g @anthropic-ai/claude-code 2>/dev/null || npm install -g @anthropic-ai/claude-code
fi
ok "claude-code installed; launch with: fcd"

# ---------- 10. daemon as a systemd user service ----------
say "10. framerize daemon (systemd --user, auto-restart)"
mkdir -p "$HOME/.config/systemd/user" "$STATE/profiles"
cat > "$HOME/.config/systemd/user/framerize.service" <<UNIT
[Unit]
Description=framerize daemon (Framer AI gateway)
After=network-online.target

[Service]
ExecStart=$HOME/.local/bin/framerize daemon --engine camoufox --profiles $STATE/profiles --db $STATE/framerize.db
Restart=on-failure
RestartSec=3
Environment=FRAMERIZE_LOG_DIR=$STATE/profiles/.logs

[Install]
WantedBy=default.target
UNIT
systemctl --user daemon-reload
systemctl --user enable --now framerize.service
loginctl enable-linger "$USER" 2>/dev/null || true
sleep 3
curl -s -m 5 http://127.0.0.1:8787/api/health && ok "daemon healthy on :8787" || warn "daemon not answering yet — check: journalctl --user -u framerize -n 30"

# ---------- 11. T3 Code (agent control surface + T3 Connect) ----------
say "11. T3 Code CLI"
if ! command -v t3 >/dev/null; then
  curl -fsSL https://t3.codes/install.sh | sh
fi
ok "t3 installed ($(t3 --version 2>/dev/null || echo '?'))"

# ---------- 12. make the agents' DEFAULT invocation ride the gateway ----------
# T3 launches `claude` / `codex` directly — so the daemon URL must be in their
# config, not only in the fcd/fcx wrappers.
say "12. gateway env in agent configs"
mkdir -p "$HOME/.claude"
python3 - <<'PYEOF2'
import json, os
p = os.path.expanduser("~/.claude/settings.json")
s = {}
if os.path.exists(p):
    try: s = json.load(open(p))
    except Exception: s = {}
env = s.setdefault("env", {})
env["ANTHROPIC_BASE_URL"] = "http://127.0.0.1:8787/anthropic"
env["ANTHROPIC_AUTH_TOKEN"] = "framerize-local"
json.dump(s, open(p, "w"), indent=2)
print("    ✓ ~/.claude/settings.json → framerize gateway")
PYEOF2
ok "claude/codex default to the free route (T3-spawned sessions included)"

say "DONE. Next steps:"
echo "    framerize login          # add your Framer account (magic link)"
echo "    fcd                      # Claude Code on the free route"
echo "    fcx                      # Codex on the free route"
echo "    t3 connect --headless    # link this server to app.t3.codes (device code)"
echo "    t3 serve                 # (or accept the background service offer)"
