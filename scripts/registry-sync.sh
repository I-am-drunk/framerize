#!/usr/bin/env bash
# framerize account-registry sync — keeps every machine's daemon on the SAME
# account pool. The registry is a private git repo holding one JSON blob per
# account (the accounts table is literally id→json) plus the session cookie
# files. Push exports the local DB; pull imports remote changes; both are HOT
# (the daemon reads accounts + sessions from disk per request — no restart).
#
# Usage: registry-sync.sh <export|import|push|pull|sync|delete <id>>
# Env overrides:
#   FRAMERIZE_DB            (default ~/.framerize/framerize.db)
#   FRAMERIZE_SESSIONS      (default ~/.framerize/profiles/.sessions)
#   FRAMERIZE_REGISTRY_DIR  (default ~/.framerize/registry — the git clone)
#   FRAMERIZE_REGISTRY_REPO (default read from ~/.framerize/.registry-repo)
#   Token: ~/.framerize/.registry-token (0600) is embedded into the https URL.
set -u
DB="${FRAMERIZE_DB:-$HOME/.framerize/framerize.db}"
SESS="${FRAMERIZE_SESSIONS:-$HOME/.framerize/profiles/.sessions}"
RDIR="${FRAMERIZE_REGISTRY_DIR:-$HOME/.framerize/registry}"
REPO_URL="${FRAMERIZE_REGISTRY_REPO:-}"

if [ -z "$REPO_URL" ]; then
  TOK="$(cat "$HOME/.framerize/.registry-token" 2>/dev/null || true)"
  [ -n "$TOK" ] && REPO_URL="https://x-access-token:${TOK}@github.com/I-am-drunk/framerize-registry.git"
fi
[ -z "$REPO_URL" ] && { echo "no registry repo configured (~/.framerize/.registry-token missing)"; exit 1; }

clone_if_needed() {
  if [ ! -d "$RDIR/.git" ]; then
    rm -rf "$RDIR"; mkdir -p "$(dirname "$RDIR")"
    git clone -q "$REPO_URL" "$RDIR" 2>/dev/null || { mkdir -p "$RDIR"; cd "$RDIR" && git init -q && git remote add origin "$REPO_URL"; }
  fi
  cd "$RDIR" && git config user.name "framerize-sync" && git config user.email "sync@framerize.local"
  mkdir -p accounts sessions
}

do_export() {
  clone_if_needed
  python3 - "$DB" "$RDIR" <<'PY'
import json, os, sqlite3, sys, shutil
db, rdir = sys.argv[1], sys.argv[2]
os.makedirs(f"{rdir}/accounts", exist_ok=True)
con = sqlite3.connect(db)
rows = con.execute("SELECT id, json FROM accounts").fetchall()
con.close()
seen = set()
for rid, blob in rows:
    seen.add(rid)
    obj = json.loads(blob)
    # Machine-local fields NEVER sync (2026-10-11, the two-fingerprints-one-cuid
    # incident): castle_device_id is the per-machine device identity — two
    # machines presenting one cuid from two browser fingerprints is an instant
    # Castle anomaly (403 on first fire). proxy is a machine-local egress
    # setting. Each machine owns these for itself.
    obj["castle_device_id"] = None
    obj["proxy"] = None
    tmp = f"{rdir}/accounts/{rid}.json.tmp"
    with open(tmp, "w") as f:
        f.write(json.dumps(obj, indent=1, sort_keys=True))
    os.replace(tmp, f"{rdir}/accounts/{rid}.json")
# remove registry files for accounts no longer present locally (tombstoned remotely stay gone)
for fn in os.listdir(f"{rdir}/accounts"):
    if fn[:-5] not in seen and fn.endswith(".json"):
        os.remove(f"{rdir}/accounts/{fn}")
PY
  # sessions (newer local mtime always exports)
  mkdir -p "$RDIR/sessions"
  if [ -d "$SESS" ]; then
    for f in "$SESS"/*.session; do
      [ -f "$f" ] || continue
      base="$(basename "$f")"
      if [ ! -f "$RDIR/sessions/$base" ] || [ "$f" -nt "$RDIR/sessions/$base" ]; then
        cp "$f" "$RDIR/sessions/$base"
      fi
    done
  fi
}

do_import() {
  clone_if_needed
  python3 - "$DB" "$RDIR" <<'PY'
import json, os, sqlite3, sys
db, rdir = sys.argv[1], sys.argv[2]
con = sqlite3.connect(db)
con.execute("CREATE TABLE IF NOT EXISTS accounts (id TEXT PRIMARY KEY, json TEXT NOT NULL)")
tomb = []
tj = os.path.join(rdir, "tombstones.json")
if os.path.exists(tj):
    tomb = json.load(open(tj))
n_add = n_del = 0
for rid in tomb:
    con.execute("DELETE FROM accounts WHERE id = ?", (rid,)); n_del += 1
adir = os.path.join(rdir, "accounts")
if os.path.isdir(adir):
    for fn in sorted(os.listdir(adir)):
        if not fn.endswith(".json"): continue
        rid = fn[:-5]
        if rid in tomb: continue
        obj = json.loads(open(os.path.join(adir, fn)).read())
        # device identity is machine-local: keep the local one on update,
        # generate a fresh one for new accounts (never inherit the remote's —
        # one cuid on two fingerprints is an instant Castle anomaly).
        local = con.execute("SELECT json FROM accounts WHERE id = ?", (rid,)).fetchone()
        if local:
            try:
                obj["castle_device_id"] = json.loads(local[0]).get("castle_device_id") or str(__import__("uuid").uuid4())
            except Exception:
                obj["castle_device_id"] = str(__import__("uuid").uuid4())
        elif not obj.get("castle_device_id"):
            obj["castle_device_id"] = str(__import__("uuid").uuid4())
        blob = json.dumps(obj)
        con.execute("INSERT INTO accounts (id, json) VALUES (?, ?) ON CONFLICT(id) DO UPDATE SET json=excluded.json", (rid, blob))
        n_add += 1
con.commit(); con.close()
print(f"import: {n_add} upserted, {n_del} tombstoned")
PY
  # sessions: remote file wins when newer (or local missing)
  if [ -d "$RDIR/sessions" ]; then
    mkdir -p "$SESS"
    for f in "$RDIR/sessions"/*.session; do
      [ -f "$f" ] || continue
      base="$(basename "$f")"
      if [ ! -f "$SESS/$base" ] || [ "$f" -nt "$SESS/$base" ]; then
        cp "$f" "$SESS/$base"; chmod 600 "$SESS/$base" 2>/dev/null || true
      fi
    done
  fi
}

case "${1:-}" in
  export) do_export ;;
  import) do_import ;;
  push)
    do_export
    cd "$RDIR"
    git add -A
    if ! git diff --cached --quiet; then
      git commit -qm "sync $(date -u +%Y-%m-%dT%H:%M:%SZ) from $(hostname)"
      git push -q origin HEAD:main 2>/dev/null || git push -q origin HEAD:master
      echo "pushed"
    else
      echo "no changes"
    fi ;;
  pull)
    clone_if_needed
    cd "$RDIR" && (git pull -q --rebase origin main 2>/dev/null || git pull -q --rebase origin master 2>/dev/null || true)
    do_import ;;
  sync)
    clone_if_needed
    cd "$RDIR" && (git pull -q --rebase origin main 2>/dev/null || git pull -q --rebase origin master 2>/dev/null || true)
    do_import
    do_export
    cd "$RDIR"
    git add -A
    if ! git diff --cached --quiet; then
      git commit -qm "sync $(date -u +%Y-%m-%dT%H:%M:%SZ) from $(hostname)"
      git push -q origin HEAD:main 2>/dev/null || git push -q origin HEAD:master
    fi ;;
  delete)
    RID="${2:?delete needs an account id}"
    python3 - "$DB" "$RID" <<'PY'
import sqlite3, sys
con = sqlite3.connect(sys.argv[1])
con.execute("DELETE FROM accounts WHERE id = ?", (sys.argv[2],))
con.commit(); con.close()
PY
    clone_if_needed
    python3 - "$RDIR" "$RID" <<'PY'
import json, os, sys
rdir, rid = sys.argv[1], sys.argv[2]
p = os.path.join(rdir, "tombstones.json")
t = json.load(open(p)) if os.path.exists(p) else []
if rid not in t: t.append(rid)
json.dump(t, open(p, "w"))
acc = os.path.join(rdir, "accounts", f"{rid}.json")
if os.path.exists(acc): os.remove(acc)
PY
    echo "tombstoned $RID (push to propagate)" ;;
  *) echo "usage: registry-sync.sh <export|import|push|pull|sync|delete <id>>"; exit 1 ;;
esac
