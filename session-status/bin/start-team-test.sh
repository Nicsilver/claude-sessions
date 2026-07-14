#!/usr/bin/env bash
#
# start-team-test.sh <count> [--oldest]
#
# Looks up the first <count> AGI issues (Bugs + Stories, like the "Ready for team test"
# YouTrack widget) currently in the "Team test" state on YouTrack
# and asks the running IntelliJ (via the Claude Sessions plugin) to open one Classic
# terminal tab per story, each running:   c "/perform-team-test AGI-xxxxx"
#
# Each spawned session auto-claims a free Playwright/Chrome lane (claim-lane.sh inside the
# perform-team-test skill), so don't request more than TEAMTEST_LANES (default 5) at once.
#
# Ordering: most-recently-updated first (default), oldest-created first with --oldest.
#
# Requires: an open IntelliJ with the Claude Sessions plugin loaded, and the YouTrack token
# stored in the keychain (service brunata-claude-code / account youtrack_token).

set -euo pipefail

YT_BASE="https://brunata.youtrack.cloud"
REQ_FILE="${TEAMTEST_REQ_FILE:-$HOME/.claude/session-status/focus-request.json}"
LANES="${TEAMTEST_LANES:-5}"

N=""
SORT="updated desc"
for arg in "$@"; do
  case "$arg" in
    --oldest)     SORT="created asc" ;;
    ''|*[!0-9]*)  ;;                       # ignore flags / non-numeric tokens
    *)            N="$arg" ;;
  esac
done

if [ -z "$N" ] || [ "$N" -lt 1 ]; then
  echo "usage: $(basename "$0") <count> [--oldest]" >&2
  exit 1
fi

if [ "$N" -gt "$LANES" ]; then
  echo "⚠️  $N requested but only $LANES Chrome lane(s) exist (TEAMTEST_LANES=$LANES);" >&2
  echo "    sessions past lane $LANES will wait for one to free up." >&2
fi

TOKEN=$(security find-generic-password -s brunata-claude-code -a youtrack_token -w 2>/dev/null || true)
if [ -z "$TOKEN" ]; then
  echo "❌ Could not read youtrack_token from keychain (service: brunata-claude-code)." >&2
  exit 1
fi

QUERY="project: AGI State: {Team test} sort by: $SORT"

RESP=$(curl -sf -G "$YT_BASE/api/issues" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Accept: application/json" \
  --data-urlencode "query=$QUERY" \
  --data-urlencode "fields=idReadable" \
  --data-urlencode "\$top=$N") || {
    echo "❌ YouTrack query failed (token expired or network down?)." >&2
    exit 1
  }

AGIS=()
while IFS= read -r line; do
  [ -n "$line" ] && AGIS+=("$line")
done < <(printf '%s' "$RESP" | python3 -c 'import sys,json; [print(i["idReadable"]) for i in json.load(sys.stdin)]')

if [ "${#AGIS[@]}" -eq 0 ]; then
  echo "No Stories in 'Team test' — nothing to do."
  exit 0
fi

echo "Opening ${#AGIS[@]} team-test session(s) in IntelliJ:"
for a in "${AGIS[@]}"; do echo "  • $a"; done

mkdir -p "$(dirname "$REQ_FILE")"
python3 - "$REQ_FILE" "${AGIS[@]}" <<'PY'
import json, os, sys, time
req_file, agis = sys.argv[1], sys.argv[2:]
payload = {"action": "team-test", "ts": time.time(),
           "cmds": [f"/perform-team-test {a}" for a in agis]}
tmp = req_file + ".tmp"
with open(tmp, "w") as f:
    json.dump(payload, f)
os.replace(tmp, req_file)
PY

echo "✅ Sent to IntelliJ — tabs should open now (make sure IntelliJ is running)."
