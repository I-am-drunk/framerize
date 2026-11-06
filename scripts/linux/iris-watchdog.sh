#!/bin/bash
# iris death-spiral watchdog — acts BEFORE the box becomes unreachable.
# Install: systemd unit runs this every 60s.
# The failure mode it kills: disk 100% or memory exhaustion → daemons die →
# tunnel + tailscale + sshd all lost → remote lockout (happened 3x in 2 days).
LOG=${FRAMERIZE_WATCHDOG_LOG:-/var/log/framerize-watchdog.log}
HEALTH_URL=${FRAMERIZE_WATCHDOG_HEALTH_URL:-http://127.0.0.1:8787/api/ready}
say() { echo "$(date -u '+%Y-%m-%d %H:%M:%S') $*" >> "$LOG"; }

# ---- 1. Disk guard: keep / under 92% by pruning ONLY regenerable caches ----
DISK_PCT=$(df / --output=pcent | tail -1 | tr -dc '0-9')
if [ "$DISK_PCT" -ge 92 ]; then
  say "DISK ${DISK_PCT}% — pruning regenerable caches"
  # apt cache (redownloadable)
  apt-get clean 2>/dev/null
  # npm cache (redownloadable)
  rm -rf /home/tjninja/.npm/_cacache 2>/dev/null
  # Firefox/Camoufox profile caches (rebuilt on next launch; profiles themselves untouched)
  find /home/tjninja/.framerize/profiles -maxdepth 4 -type d \( -name cache2 -o -name 'startupCache' -o -name 'thumbnails' \) -exec rm -rf {} + 2>/dev/null
  find /home/tjninja/.cache/camoufox -maxdepth 6 -type d -name cache2 -exec rm -rf {} + 2>/dev/null
  # journal vacuum to 200M
  journalctl --vacuum-size=200M >/dev/null 2>&1
  # old /tmp debris (never session state: .sessions, devices.json, profiles are untouched)
  find /tmp -maxdepth 1 -name 'tmp*' -mtime +1 -delete 2>/dev/null
  DISK_AFTER=$(df / --output=pcent | tail -1 | tr -dc '0-9')
  say "DISK after prune: ${DISK_AFTER}%"
fi

# ---- 2. Memory guard: restart only when low memory coincides with failed readiness ----
AVAIL_MB=$(free -m | awk '/^Mem:/ {print $7}')
if [ "$AVAIL_MB" -lt 300 ]; then
  if curl --connect-timeout 2 --max-time 4 -fsS "$HEALTH_URL" >/dev/null 2>&1; then
    say "MEM ${AVAIL_MB}MB available but Framerize is ready — no restart"
  else
    say "MEM ${AVAIL_MB}MB available and Framerize is not ready — requesting graceful restart"
    if systemctl --user --machine=tjninja@ restart framerize.service 2>/dev/null \
      || runuser -l tjninja -c 'XDG_RUNTIME_DIR=/run/user/1000 systemctl --user restart framerize.service' 2>/dev/null; then
      ready=0
      for _ in $(seq 1 15); do
        if curl --connect-timeout 2 --max-time 4 -fsS "$HEALTH_URL" >/dev/null 2>&1; then
          ready=1
          break
        fi
        sleep 2
      done
      if [ "$ready" -eq 1 ]; then
        say "Framerize readiness recovered after restart; memory available $(free -m | awk '/^Mem:/ {print $7}')MB"
      else
        say "Framerize remains unready after restart; memory available $(free -m | awk '/^Mem:/ {print $7}')MB"
      fi
    else
      say "Framerize restart command failed; service state was preserved"
    fi
  fi
fi

# ---- 3. sshd liveness: if port 22 stops answering locally, restart it ----
if ! timeout 3 bash -c 'echo > /dev/tcp/127.0.0.1/22' 2>/dev/null; then
  say "SSHD unresponsive — restarting"
  systemctl restart ssh 2>/dev/null || systemctl restart sshd 2>/dev/null
fi

# ---- 4. tunnel liveness: if cloudflared died and systemd hasn't revived it, kick it ----
if ! pgrep -f 'cloudflared tunnel' >/dev/null; then
  say "cloudflared not running — restarting tunnel service"
  systemctl restart cloudflared-tunnel 2>/dev/null
fi
