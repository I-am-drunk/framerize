#!/usr/bin/env bash
# framerize daemon lifecycle manager — PID-file based. NEVER uses pkill -f
# (a -f pattern appears in the wrapper shell's own cmdline and kills it).
set -u
ROOT="${FRAMERIZE_ROOT:-/workspace}"
BIN=$ROOT/framerize/target/debug/framerize
CHROME=$ROOT/chrome-headless-shell-linux64/chrome-headless-shell
PIDFILE=$ROOT/daemon.pid
LOG=$ROOT/daemon.log
DB=$ROOT/test-framerize.db
PROFILES=$ROOT/profiles
PORT=8787

case "${1:-}" in
  start)
    if [ -f "$PIDFILE" ] && kill -0 "$(cat $PIDFILE)" 2>/dev/null; then
      echo "already running pid $(cat $PIDFILE)"; exit 0
    fi
    setsid bash -c "exec '$BIN' daemon --listen 127.0.0.1:$PORT --db '$DB' --chrome '$CHROME' --profiles '$PROFILES'" \
      < /dev/null > "$LOG" 2>&1 &
    echo $! > "$PIDFILE"
    echo "started pid $(cat $PIDFILE)"
    ;;
  stop)
    if [ -f "$PIDFILE" ]; then
      PID=$(cat "$PIDFILE")
      # kill the process group (daemon), then any chrome children by exact comm
      kill -- -"$PID" 2>/dev/null; kill "$PID" 2>/dev/null
      rm -f "$PIDFILE"
      echo "stopped $PID"
    else
      echo "not running"
    fi
    # chrome cleanup: exact comm match only (15-char kernel truncation)
    pkill -x chrome-headless 2>/dev/null && echo "chrome cleaned" || true
    ;;
  status)
    if [ -f "$PIDFILE" ] && kill -0 "$(cat $PIDFILE)" 2>/dev/null; then
      echo "running pid $(cat $PIDFILE)"
    else
      echo "not running"
    fi
    pgrep -x chrome-headless | wc -l | xargs echo "chrome procs:"
    free -m | awk 'NR==2{print "mem free MB:", $4}'
    ;;
  health)
    curl -s -m 4 "http://127.0.0.1:$PORT/api/health" || echo "NO-RESPONSE"
    ;;
  *)
    echo "usage: $0 {start|stop|status|health}"; exit 1;;
esac
