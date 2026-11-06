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
STOP_TIMEOUT_S=${FRAMERIZE_STOP_TIMEOUT_S:-300}

is_our_daemon() {
  local pid=$1 group command
  [[ "$pid" =~ ^[0-9]+$ ]] && kill -0 "$pid" 2>/dev/null || return 1
  group=$(ps -o pgid= -p "$pid" 2>/dev/null | tr -d ' ')
  command=$(ps -o args= -p "$pid" 2>/dev/null)
  [ "$group" = "$pid" ] || return 1
  case "$command" in *"$BIN"*) return 0 ;; esac
  return 1
}

case "${1:-}" in
  start)
    if [ -f "$PIDFILE" ]; then
      PID=$(cat "$PIDFILE")
      if is_our_daemon "$PID"; then
        echo "already running pid $PID"; exit 0
      elif kill -0 -- -"$PID" 2>/dev/null; then
        echo "stale daemon leader has a remaining process group; refusing to start"; exit 1
      elif kill -0 "$PID" 2>/dev/null; then
        echo "pid file points to a different process; refusing to start"; exit 1
      else
        rm -f "$PIDFILE"
      fi
    fi
    setsid bash -c "exec '$BIN' daemon --listen 127.0.0.1:$PORT --db '$DB' --chrome '$CHROME' --profiles '$PROFILES'" \
      < /dev/null > "$LOG" 2>&1 &
    echo $! > "$PIDFILE"
    echo "started pid $(cat $PIDFILE)"
    ;;
  stop)
    [ -f "$PIDFILE" ] || { echo "not running"; exit 0; }
    PID=$(cat "$PIDFILE")
    if ! kill -0 "$PID" 2>/dev/null; then
      if kill -0 -- -"$PID" 2>/dev/null; then
        echo "daemon leader exited but its process group remains; pid file retained"; exit 1
      fi
      rm -f "$PIDFILE"
      echo "removed stale pid file"
      exit 0
    fi
    if ! is_our_daemon "$PID"; then
      echo "pid file does not identify this daemon; refusing to signal it"; exit 1
    fi
    if ! [[ "$STOP_TIMEOUT_S" =~ ^[0-9]+$ ]]; then
      echo "FRAMERIZE_STOP_TIMEOUT_S must be a non-negative integer"; exit 1
    fi
    # The daemon and its browser children share this isolated process group.
    # Keep the PID file until every member exits; never kill unrelated browsers.
    kill -TERM -- -"$PID" 2>/dev/null || true
    deadline=$((SECONDS + STOP_TIMEOUT_S))
    while kill -0 -- -"$PID" 2>/dev/null; do
      if [ "$SECONDS" -ge "$deadline" ]; then
        echo "daemon group $PID is still stopping; pid file retained"; exit 1
      fi
      sleep 0.2
    done
    rm -f "$PIDFILE"
    echo "stopped $PID"
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
