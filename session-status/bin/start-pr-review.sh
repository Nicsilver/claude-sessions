#!/usr/bin/env bash
#
# start-pr-review.sh <count> [--oldest]
#
# Finds AGI issues currently in "Code review" on YouTrack (the review queue — same idea as
# the team-test widget), resolves their open GitHub PRs in the team-webbill org, and asks the
# running IntelliJ (via the Claude Sessions plugin) to open one Classic terminal tab per PR,
# each running:   c "/pr-review <pr-url>"
#
# Skips, per request:
#   • draft PRs
#   • PRs you authored (your own work)
#   • PRs that have unresolved review-comment threads
# (pr-review additionally self-skips merged/closed/automated/already-reviewed PRs.)
#
# Ordering: most-recently-updated issue first (default), oldest-created first with --oldest.
# Requires: open IntelliJ w/ Claude Sessions plugin, `gh` authed, YouTrack token in keychain.

set -euo pipefail

YT_BASE="https://brunata.youtrack.cloud"
REQ_FILE="${TEAMTEST_REQ_FILE:-$HOME/.claude/session-status/focus-request.json}"
OWNER="team-webbill"

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

TOKEN=$(security find-generic-password -s brunata-claude-code -a youtrack_token -w 2>/dev/null || true)
if [ -z "$TOKEN" ]; then
  echo "❌ Could not read youtrack_token from keychain (service: brunata-claude-code)." >&2
  exit 1
fi
ME=$(gh api user --jq .login 2>/dev/null || true)
if [ -z "$ME" ]; then
  echo "❌ gh is not authenticated (run: gh auth status)." >&2
  exit 1
fi

QUERY="project: AGI State: {Code review} sort by: $SORT"
# Scan more issues than N — most rows get filtered out (subtasks share the parent's PR, plus
# drafts / your own / unresolved-comment PRs), so over-fetch and stop once N PRs are found.
SCAN=$(( N * 10 )); [ "$SCAN" -lt 50 ] && SCAN=50

RESP=$(curl -sf -G "$YT_BASE/api/issues" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Accept: application/json" \
  --data-urlencode "query=$QUERY" \
  --data-urlencode "fields=idReadable" \
  --data-urlencode "\$top=$SCAN") || {
    echo "❌ YouTrack query failed (token expired or network down?)." >&2
    exit 1
  }

KEYS=()
while IFS= read -r k; do [ -n "$k" ] && KEYS+=("$k"); done < <(
  printf '%s' "$RESP" | python3 -c 'import sys,json; [print(i["idReadable"]) for i in json.load(sys.stdin)]')

if [ "${#KEYS[@]}" -eq 0 ]; then
  echo "No issues in 'Code review' — nothing to do."
  exit 0
fi
echo "Scanning ${#KEYS[@]} 'Code review' issue(s) for reviewable PRs (need $N)…"

# Count unresolved review-comment threads on a PR (GitHub GraphQL).
unresolved_count() {  # owner repo number
  gh api graphql -f query='
    query($o:String!,$r:String!,$n:Int!){
      repository(owner:$o,name:$r){ pullRequest(number:$n){
        reviewThreads(first:100){ nodes{ isResolved } } } } }' \
    -F o="$1" -F r="$2" -F n="$3" \
    --jq '[.data.repository.pullRequest.reviewThreads.nodes[] | select(.isResolved==false)] | length' \
    2>/dev/null || echo 0
}

PR_URLS=()
SEEN=" "
for KEY in "${KEYS[@]}"; do
  if [ "${#PR_URLS[@]}" -ge "$N" ]; then break; fi
  # open PRs whose TITLE matches this key (team convention: title starts with the parent AGI key)
  while IFS=$'\t' read -r url isdraft author nwo num title; do
    if [ "${#PR_URLS[@]}" -ge "$N" ]; then break; fi
    [ -n "$url" ] || continue
    case "$SEEN" in *" $url "*) continue ;; esac                  # dedupe across keys
    case "$title" in "$KEY "*|"$KEY:"*) : ;; *) continue ;; esac   # exact-key guard (avoid token false matches)
    SEEN="$SEEN$url "
    if [ "$isdraft" = "true" ]; then echo "  – skip $url ($KEY, draft)"; continue; fi
    if [ "$author" = "$ME" ];   then echo "  – skip $url ($KEY, your own PR)"; continue; fi
    owner="${nwo%%/*}"; repo="${nwo#*/}"
    u=$(unresolved_count "$owner" "$repo" "$num")
    if [ "${u:-0}" -gt 0 ]; then echo "  – skip $url ($KEY, $u unresolved comment thread(s))"; continue; fi
    echo "  ✓ $KEY → $url"
    PR_URLS+=("$url")
  done < <(gh search prs "$KEY" --owner "$OWNER" --state open --match title \
             --json url,isDraft,author,repository,number,title \
             --jq '.[] | [.url,(.isDraft|tostring),.author.login,.repository.nameWithOwner,(.number|tostring),.title] | @tsv' \
           2>/dev/null)
done

if [ "${#PR_URLS[@]}" -eq 0 ]; then
  echo "No reviewable PRs found (after skipping drafts, your own, and unresolved-comment PRs)."
  exit 0
fi

echo "Opening ${#PR_URLS[@]} PR-review session(s) in IntelliJ:"
for u in "${PR_URLS[@]}"; do echo "  • $u"; done

mkdir -p "$(dirname "$REQ_FILE")"
python3 - "$REQ_FILE" "${PR_URLS[@]}" <<'PY'
import json, os, sys, time
req_file, urls = sys.argv[1], sys.argv[2:]
payload = {"action": "team-test", "ts": time.time(),
           "cmds": [f"/pr-review {u}" for u in urls]}
tmp = req_file + ".tmp"
with open(tmp, "w") as f:
    json.dump(payload, f)
os.replace(tmp, req_file)
PY

echo "✅ Sent to IntelliJ — review tabs should open now (make sure IntelliJ is running)."
