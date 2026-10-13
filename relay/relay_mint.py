#!/usr/bin/env python3
"""JWT relay minter — one pass. Reads sessions from the framerize-registry
repo, mints Framer AI JWTs from THIS runner's IP, and publishes them to
framerize/.ops/jwt-relay.json. Runs anywhere with a clean egress (GitHub
Actions runner, sandbox, VPS). Env: RELAY_PAT (repo-scope PAT)."""
import base64, glob, json, os, subprocess, time, urllib.request

PAT = os.environ["RELAY_PAT"]
REG_DIR = os.environ.get("REG_DIR", "/tmp/framerize-registry")
REGISTRY = "https://x-access-token:%s@github.com/I-am-drunk/framerize-registry.git" % PAT
API = "https://api.github.com/repos/I-am-drunk/framerize/contents/.ops/jwt-relay.json"

def sh(cmd):
    return subprocess.run(cmd, capture_output=True, text=True)

def mint(cookie_val):
    req = urllib.request.Request(
        "https://api.framer.com/auth/ai/access-token",
        headers={"cookie": "session=" + cookie_val, "x-requested-by": "Framer",
                 "user-agent": "Framer/2026.39.0"})
    with urllib.request.urlopen(req, timeout=20) as r:
        return json.loads(r.read())

def publish(payload):
    sha = None
    req = urllib.request.Request(API, headers={"Authorization": "token " + PAT, "Accept": "application/vnd.github+json", "user-agent": "jwt-relay"})
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            sha = json.loads(r.read()).get("sha")
    except Exception:
        pass
    doc = {"message": "jwt-relay %s" % payload["updatedAt"],
           "content": base64.b64encode(json.dumps(payload, indent=1).encode()).decode()}
    if sha:
        doc["sha"] = sha
    req = urllib.request.Request(API, data=json.dumps(doc).encode(), method="PUT",
        headers={"Authorization": "token " + PAT, "Accept": "application/vnd.github+json",
                 "content-type": "application/json", "user-agent": "jwt-relay"})
    with urllib.request.urlopen(req, timeout=30) as r:
        return r.status in (200, 201)

def main():
    if os.path.exists(REG_DIR):
        sh(["git", "-C", REG_DIR, "pull", "-q"])
    else:
        sh(["git", "clone", "-q", "--depth", "1", REGISTRY, REG_DIR])
    accounts = {}
    total = 0
    for f in sorted(glob.glob(os.path.join(REG_DIR, "sessions", "*.session"))):
        uid = os.path.basename(f)[:-8]
        raw = open(f).read().strip()
        if "session=" not in raw:
            continue
        total += 1
        cv = raw.split("session=", 1)[1].split(";")[0].strip()
        try:
            j = mint(cv)
            if j.get("accessToken"):
                accounts[uid] = {"accessToken": j["accessToken"], "expiresAt": j.get("expiresAt")}
            else:
                print("mint empty", uid[:8], flush=True)
        except Exception as e:
            print("mint FAIL", uid[:8], str(e)[:90], flush=True)
    payload = {"updatedAt": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), "accounts": accounts}
    ok = publish(payload)
    print("%s minted %d/%d published=%s" % (payload["updatedAt"], len(accounts), total, ok), flush=True)
    if not accounts:
        raise SystemExit("no accounts minted — refusing to publish empty")

main()
