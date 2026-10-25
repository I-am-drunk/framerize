#!/usr/bin/env bash
# Compatibility entry point for staging a complete release archive.
set -euo pipefail
repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
if [ "$#" -eq 0 ]; then
  echo 'Usage: server-setup.sh ARCHIVE --sha256 CHECKSUM [--releases DIRECTORY]' >&2
  echo 'Stages a release without changing packages, accounts, or running services.' >&2
  exit 2
fi
exec python3 "$repo/scripts/release.py" stage "$@"
