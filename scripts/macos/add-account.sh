#!/usr/bin/env bash
# add-account.sh <email> [project-id] — magic-link onboarding.
set -euo pipefail
EMAIL="${1:?usage: add-account.sh <email> [project-id]}"
PROJECT="${2:-}"
BASE="${FRAMERIZE_URL:-http://127.0.0.1:8787}"

# uuidgen isn't installed everywhere; the kernel has one.
ACCT=$(cat /proc/sys/kernel/random/uuid 2>/dev/null || uuidgen)
echo "account id: $ACCT"
echo "==> sending magic link to $EMAIL"
curl -s -X POST "$BASE/api/accounts/begin" -H 'content-type: application/json' \
  -d "{\"account_id\":\"$ACCT\",\"email\":\"$EMAIL\"}"
echo; echo
read -rp "Paste the #token= UUID from the emailed link: " TOKEN
BODY="{\"account_id\":\"$ACCT\",\"email_token\":\"$TOKEN\",\"alias\":\"$EMAIL\""
[ -n "$PROJECT" ] && BODY="$BODY,\"project_id\":\"$PROJECT\""
BODY="$BODY}"
curl -s -X POST "$BASE/api/accounts/complete" -H 'content-type: application/json' -d "$BODY"
echo
