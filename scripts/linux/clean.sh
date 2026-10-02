#!/usr/bin/env bash
# framerize cleanup — for space-constrained machines.
# Levels: ./clean.sh          → removes accidental nested clones + temp junk
#         ./clean.sh --hard   → also strips target/ intermediates (keeps the
#                               built binary by installing it to ~/.local/bin first)
set -euo pipefail
cd "$(dirname "$0")/../.."

freed() { du -sh . 2>/dev/null | awk '{print "repo size now:", $1}'; }

echo "==> removing accidental nested clones (framerize/framerize)"
[ -d framerize/.git ] && rm -rfv framerize && echo "nested clone deleted" || echo "none found"

echo "==> temp junk"
rm -f /tmp/chs.zip nohup.out 2>/dev/null || true
cargo clean --doc 2>/dev/null || true   # doc artifacts only; harmless

freed

if [ "${1:-}" = "--hard" ]; then
  echo "==> HARD clean: installing the binary, then wiping target/"
  mkdir -p "$HOME/.local/bin"
  [ -x target/release/framerize ] && cp -v target/release/framerize "$HOME/.local/bin/framerize"
  [ -x target/debug/framerize ] && [ ! -x target/release/framerize ] && cp -v target/debug/framerize "$HOME/.local/bin/framerize"
  rm -rf target
  echo "binary at ~/.local/bin/framerize (rebuilds from scratch next time)"
fi
freed
