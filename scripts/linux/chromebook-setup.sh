#!/usr/bin/env bash
# framerize Chromebook setup — one command does everything.
# Usage: bash scripts/linux/chromebook-setup.sh
# Idempotent: safe to re-run; skips what's already done.
set -euo pipefail

PAT="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"
REPO="I-am-drunk/framerize"
STATE="$HOME/.framerize"
ALIASES=(concise_moodiness571 evasion_maximum486 payphone_freestyle462 broadband_fiddling610 exes_slashed533)
NAMES=(reis evasion payphone broadband exes)

say() { printf '\n\033[1;36m==> %s\033[0m\n' "$*"; }
ok()  { printf '\033[1;32m  ok\033[0m %s\n' "$*"; }
warn(){ printf '\033[1;33m  !!\033[0m %s\n' "$*"; }

cd "$(dirname "$0")/../.."   # repo root

# ── 1. latest code ────────────────────────────────────────────────
say "repo"
git pull -q origin main && ok "code up to date ($(git log --oneline -1 | cut -c1-7))"

# ── 2. framerize binary (latest GitHub release, no compile) ──────
say "framerize binary"
mkdir -p "$HOME/.local/bin"
ASSET_ID=$(curl -s -H "Authorization: token $PAT" \
  "https://api.github.com/repos/$REPO/releases/latest" \
  | python3 -c "import json,sys; r=json.load(sys.stdin); print(next(a['id'] for a in r['assets'] if a['name']=='framerize-linux-x86_64'))")
curl -sL -H "Authorization: token $PAT" -H "Accept: application/octet-stream" \
  -o "$HOME/.local/bin/framerize" \
  "https://api.github.com/repos/$REPO/releases/assets/$ASSET_ID"
chmod +x "$HOME/.local/bin/framerize"
hash -r 2>/dev/null || true
"$HOME/.local/bin/framerize" --help > /dev/null && ok "binary installed ($("$HOME/.local/bin/framerize" --version 2>/dev/null || echo latest-release))"

# ── 3. camoufox engine ────────────────────────────────────────────
say "camoufox engine"
if [ -x "$HOME/.cache/camoufox/browsers/official/152.0.4-beta.31-3a7958c8/camoufox" ]; then
  ok "already installed"
else
  warn "downloading camoufox (~650MB, resumable)"
  mkdir -p "$HOME/.cache/camoufox"
  CAM_ID=$(curl -s -H "Authorization: token $PAT" \
    "https://api.github.com/repos/$REPO/releases/tags/v0.4.0" \
    | python3 -c "import json,sys; r=json.load(sys.stdin); print(next(a['id'] for a in r['assets'] if a['name']=='camoufox-cache.tar.gz'))")
  curl -sL -C - --retry 12 --retry-all-errors \
    -H "Authorization: token $PAT" -H "Accept: application/octet-stream" \
    -o /tmp/camoufox-cache.tar.gz \
    "https://api.github.com/repos/$REPO/releases/assets/$CAM_ID"
  echo "5a889014ccee518149bd7d082bed7a48c8598be6d158e817c47ce8a01db98dd9  /tmp/camoufox-cache.tar.gz" | sha256sum -c - \
    || { echo "checksum FAILED — delete /tmp/camoufox-cache.tar.gz and re-run"; exit 1; }
  tar -xzf /tmp/camoufox-cache.tar.gz -C "$HOME/.cache/camoufox/" && rm /tmp/camoufox-cache.tar.gz
  ok "camoufox installed"
fi
# runtime deps (best-effort; already present on most systems)
sudo apt-get install -y -qq libgtk-3-0 libx11-xcb1 libdbus-glib-1-2 libxt6 libasound2 \
  libxcomposite1 libxdamage1 libxrandr2 libxkbcommon0 libpango-1.0-0 libcairo2 python3-venv 2>/dev/null || true
# python venv for the sidecar
if [ ! -x "$STATE/venv/bin/python" ]; then
  python3 -m venv "$STATE/venv"
  "$STATE/venv/bin/pip" install -q playwright camoufox 2>/dev/null || "$STATE/venv/bin/pip" install -q playwright camoufox
fi
ok "sidecar venv ready"

# ── 4. agent configs (T3-spawned agents ride the free gateway) ────
say "agent configs"
mkdir -p "$HOME/.claude" "$HOME/.codex"
cat > "$HOME/.claude/settings.json" <<'EOF'
{
  "env": {
    "ANTHROPIC_BASE_URL": "http://127.0.0.1:8787/anthropic",
    "ANTHROPIC_AUTH_TOKEN": "local"
  }
}
EOF
cat > "$HOME/.codex/config.toml" <<'EOF'
model = "gpt-6-astra"
model_provider = "framerize"

[model_providers.framerize]
name = "framerize"
base_url = "http://127.0.0.1:8787/openai/v1"
wire_api = "responses"
env_key = "FRAMERIZE_LOCAL"
EOF
grep -q FRAMERIZE_LOCAL "$HOME/.bashrc" 2>/dev/null || echo 'export FRAMERIZE_LOCAL=local' >> "$HOME/.bashrc"
export FRAMERIZE_LOCAL=local
ok "claude + codex configured for the gateway"

# ── 5. daemon ─────────────────────────────────────────────────────
say "daemon"
pkill -x framerize 2>/dev/null || true
sleep 1
mkdir -p "$STATE/profiles"
nohup "$HOME/.local/bin/framerize" daemon --engine camoufox \
  --profiles "$STATE/profiles" --db "$STATE/framerize.db" \
  > "$STATE/daemon.log" 2>&1 &
sleep 3
curl -s -m 5 http://127.0.0.1:8787/api/health > /dev/null && ok "daemon up at http://127.0.0.1:8787 (log: $STATE/daemon.log)" \
  || { echo "daemon failed — last log lines:"; tail -5 "$STATE/daemon.log"; exit 1; }

# ── 6. accounts ───────────────────────────────────────────────────
say "accounts"
EXISTING=$(curl -s http://127.0.0.1:8787/api/accounts | python3 -c "import json,sys; print(' '.join(a['alias'] for a in json.load(sys.stdin)))" 2>/dev/null || echo "")
for i in "${!ALIASES[@]}"; do
  email="${ALIASES[$i]}@dralias.com"; alias="${NAMES[$i]}"
  if echo "$EXISTING" | grep -qw "$alias"; then ok "$alias already added — skipping"; continue; fi
  echo
  echo "── $alias ($email) ──"
  acct=$(cat /proc/sys/kernel/random/uuid)
  resp=$(curl -s -m 240 -X POST http://127.0.0.1:8787/api/accounts/begin \
    -H 'content-type: application/json' \
    -d "{\"account_id\":\"$acct\",\"email\":\"$email\"}")
  echo "$resp" | grep -qi 'check-email\|sent\|ok' && ok "magic link sent" || { warn "begin said: $resp"; }
  echo ">> open Gmail, find the NEWEST Framer email to $email, copy the link"
  read -r -p "   paste the #token= UUID (or the whole link): " tok
  tok="${tok##*token=}"; tok="${tok%%[& ]*}"
  curl -s -m 240 -X POST http://127.0.0.1:8787/api/accounts/complete \
    -H 'content-type: application/json' \
    -d "{\"account_id\":\"$acct\",\"email_token\":\"$tok\",\"alias\":\"$alias\"}"
  echo
done
curl -s http://127.0.0.1:8787/api/accounts | python3 -m json.tool | grep -E '"alias"|"state"|"availableCredits"' || true

# ── 7. T3 Code ────────────────────────────────────────────────────
say "T3 Code"
if ! command -v t3 > /dev/null 2>&1; then
  curl -fsSL https://t3.codes/install.sh | bash
  hash -r 2>/dev/null || true
fi
command -v t3 > /dev/null 2>&1 && ok "t3 $(t3 --version 2>/dev/null | head -1)" || warn "t3 install needs a new shell — run: source ~/.bashrc"

cat <<'NEXT'

All done. Last step — connect this machine to your T3 account:

  t3 connect          # prints a URL + code — open the URL (any device), sign in, approve
  t3 serve            # then leave this running; the environment appears in app.t3.codes

Useful:
  framerize accounts            # pool status
  framerize logs --credits      # per-request credit proof
  tail -f ~/.framerize/daemon.log
NEXT
