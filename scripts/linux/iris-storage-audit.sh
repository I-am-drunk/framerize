#!/bin/bash
# iris storage audit — finds what's eating the disk, ranked, with a keep/delete verdict.
# Rule from the operator: NEVER touch images/photos/videos or documents. Old useless
# junk (caches, logs, build artifacts, old downloads, stale node_modules) goes.
echo "=== disk ==="; df -h / | tail -1
echo
echo "=== top 25 dirs under /home/tjninja (depth 2) ==="
sudo du -x -d2 -h /home/tjninja 2>/dev/null | sort -rh | head -25
echo
echo "=== top 15 system dirs ==="
sudo du -x -d1 -h /var /usr /opt /snap 2>/dev/null | sort -rh | head -15
echo
echo "=== known junk candidates (sizes) ==="
for d in \
  /home/tjninja/.npm \
  /home/tjninja/.cache \
  /home/tjninja/.cargo/registry \
  /home/tjninja/.rustup \
  /home/tjninja/t3code \
  /home/tjninja/framerize/target \
  /home/tjninja/Downloads \
  /home/tjninja/.framerize/profiles \
  /var/cache/apt \
  /var/log \
  /var/lib/snapd/snaps \
  /home/tjninja/.framerize/venv \
  ; do
  [ -e "$d" ] && sudo du -sh "$d" 2>/dev/null
done
echo
echo "=== files >500M anywhere in home (excl. .framerize profiles) ==="
sudo find /home/tjninja -xdev -type f -size +500M -not -path '*/.framerize/profiles/*' -printf '%s %p\n' 2>/dev/null | sort -rn | head -15 | awk '{printf "%.1fG %s\n", $1/1073741824, $2}'
echo
echo "=== oldest large dirs (untouched >1yr, junk candidates) ==="
sudo find /home/tjninja -xdev -maxdepth 2 -type d -mtime +365 -printf '%T+ %p\n' 2>/dev/null | sort | head -10
