#!/bin/bash
# iris: one-shot persistent access installer. Run on the server:
#   curl -sL -H "Authorization: token <PAT>" https://raw.githubusercontent.com/I-am-drunk/framerize/main/scripts/linux/iris-access-install.sh | bash
set -e
PAT="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"
echo "==> cloudflared"
command -v cloudflared >/dev/null || { curl -sL -o /tmp/cf.deb https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64.deb && sudo dpkg -i /tmp/cf.deb; }
echo "==> ops repo"
mkdir -p ~/.framerize-ops
[ -d ~/.framerize-ops/repo/.git ] || git clone -q "https://x-access-token:${PAT}@github.com/I-am-drunk/framerize.git" ~/.framerize-ops/repo
cd ~/.framerize-ops/repo && git pull -q origin main || true

echo "==> reporter script"
cat > ~/.framerize-ops/tunnel-report.sh <<'EOS'
#!/bin/bash
REPO=/home/tjninja/.framerize-ops/repo
LOG=/var/log/cloudflared-tunnel.log
STATE=/home/tjninja/.framerize-ops/last-url
while true; do
  URL=$(grep -oE 'https://[a-z0-9-]+\.trycloudflare\.com' "$LOG" 2>/dev/null | tail -1)
  if [ -n "$URL" ] && [ "$URL" != "$(cat "$STATE" 2>/dev/null)" ]; then
    cd "$REPO" || { sleep 30; continue; }
    git pull -q origin main >/dev/null 2>&1
    mkdir -p .ops; echo "$URL" > .ops/iris-tunnel
    if ! git diff --quiet .ops/iris-tunnel 2>/dev/null; then
      git add .ops/iris-tunnel
      if git -c user.name=iris-ops -c user.email=ops@iris.local commit -qm "ops: tunnel url $URL" && git push -q origin main; then
        echo "$URL" > "$STATE"
      fi
    else
      echo "$URL" > "$STATE"
    fi
  fi
  sleep 30
done
EOS
chmod +x ~/.framerize-ops/tunnel-report.sh

echo "==> systemd units"
sudo tee /etc/systemd/system/cloudflared-tunnel.service >/dev/null <<'EOS'
[Unit]
Description=cloudflared quick tunnel (ssh ingress)
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=/usr/bin/cloudflared tunnel --url ssh://localhost:22 --no-autoupdate --logfile /var/log/cloudflared-tunnel.log
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
EOS
sudo tee /etc/systemd/system/tunnel-report.service >/dev/null <<'EOS'
[Unit]
Description=report tunnel URL to framerize repo
After=cloudflared-tunnel.service

[Service]
ExecStart=/bin/bash /home/tjninja/.framerize-ops/tunnel-report.sh
Restart=always
RestartSec=10
User=tjninja

[Install]
WantedBy=multi-user.target
EOS
sudo touch /var/log/cloudflared-tunnel.log && sudo chmod 644 /var/log/cloudflared-tunnel.log
sudo systemctl daemon-reload
sudo systemctl enable --now cloudflared-tunnel tunnel-report
sleep 15
echo "==> status:"
sudo systemctl is-active cloudflared-tunnel tunnel-report
grep -oE 'https://[a-z0-9-]+\.trycloudflare\.com' /var/log/cloudflared-tunnel.log | head -1
echo "DONE — the URL above is pushed to the repo automatically. You can close this terminal."
