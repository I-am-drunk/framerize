#!/bin/bash
# Install the iris watchdog as a system timer (every 60s) + enable zram swap.
set -e
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

echo "==> watchdog script"
sudo install -m755 "$SCRIPT_DIR/iris-watchdog.sh" /usr/local/bin/iris-watchdog.sh

echo "==> systemd timer"
sudo tee /etc/systemd/system/iris-watchdog.service >/dev/null <<'EOS'
[Unit]
Description=framerize iris death-spiral watchdog
[Service]
Type=oneshot
ExecStart=/usr/local/bin/iris-watchdog.sh
EOS
sudo tee /etc/systemd/system/iris-watchdog.timer >/dev/null <<'EOS'
[Unit]
Description=run iris watchdog every 60s
[Timer]
OnBootSec=60
OnUnitActiveSec=60
[Install]
WantedBy=timers.target
EOS
sudo systemctl daemon-reload
sudo systemctl enable --now iris-watchdog.timer

echo "==> zram swap (compressed RAM — ~2x effective memory for browser workloads)"
if ! swapon --show | grep -q zram; then
  sudo apt-get install -y zram-config 2>/dev/null || true
  # Fallback: manual zram if the package isn't available
  if ! swapon --show | grep -q zram; then
    sudo modprobe zram
    echo $((8*1024*1024*1024)) | sudo tee /sys/block/zram0/disksize >/dev/null
    sudo mkswap /dev/zram0 >/dev/null
    sudo swapon -p 100 /dev/zram0
    echo 'KERNEL=="zram0", ATTR{disksize}="8G", RUN+="/sbin/mkswap /dev/zram0", RUN+="/sbin/swapon -p 100 /dev/zram0"' | sudo tee /etc/udev/rules.d/99-zram.rules >/dev/null
  fi
fi

echo "==> fix the live tunnel reporter (head -1 reads oldest URL; tail -1 is correct)"
sudo sed -i 's|trycloudflare\\.com" "$LOG" 2>/dev/null | head -1|trycloudflare\\.com" "$LOG" 2>/dev/null | tail -1|' /home/tjninja/.framerize-ops/tunnel-report.sh 2>/dev/null \
  || sudo sed -i 's|head -1)$|tail -1)|' /home/tjninja/.framerize-ops/tunnel-report.sh 2>/dev/null || true

echo "==> done"
sudo systemctl list-timers iris-watchdog.timer --no-pager | tail -2
swapon --show | tail -2
