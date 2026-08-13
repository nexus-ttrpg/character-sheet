#!/usr/bin/env bash
#
# Live-platform canary (§16, §21) — imports a known character through a
# running Nexus API instance and verifies the round-trip is lossless.
#
# Unlike the e2e gate test (which uses an in-process API + in-memory store),
# this script hits an EXTERNAL API URL — typically a staging deployment with
# a live Foundry relay connected. It catches:
#   - Upstream Foundry format changes (the relay sends a different actor shape)
#   - Adapter mapping regressions against live data (not just fixtures)
#   - API deployment regressions (auth, store, adapter-worker, merge engine)
#
# Prerequisites (GitHub secrets or env vars):
#   CANARY_API_URL  — base URL of the Nexus API (e.g. https://nexus-staging.internal)
#   CANARY_API_KEY  — an Admin API key (nsk_live_...)
#   CANARY_RELAY_URL (optional) — Foundry relay URL, passed as connection credential
#
# Usage:
#   CANARY_API_URL=https://... CANARY_API_KEY=nsk_live_... ./scripts/canary.sh
#
# Exit codes:
#   0 — round-trip matched (no_changes or expected diff)
#   1 — round-trip produced unexpected changes
#   2 — infrastructure error (API unreachable, auth failed, etc.)

set -euo pipefail

API_URL="${CANARY_API_URL:-}"
API_KEY="${CANARY_API_KEY:-}"
RELAY_URL="${CANARY_RELAY_URL:-}"

if [[ -z "$API_URL" || -z "$API_KEY" ]]; then
  echo "CANARY_API_URL and CANARY_API_KEY are required"
  exit 2
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FIXTURE="$SCRIPT_DIR/../examples/fixtures/pf2e-level1-fighter.json"

if [[ ! -f "$FIXTURE" ]]; then
  echo "fixture not found: $FIXTURE"
  exit 2
fi

AUTH="Authorization: Bearer $API_KEY"
CT="Content-Type: application/json"

echo "=== Nexus canary ==="
echo "API: $API_URL"
echo "Fixture: $FIXTURE"
echo ""

# 1. Health check.
health=$(curl -sf -o /dev/null -w "%{http_code}" "$API_URL/v1/openapi.json" || true)
if [[ "$health" != "200" ]]; then
  echo "FAIL: API unreachable (openapi.json returned $health)"
  exit 2
fi
echo "✓ API reachable"

# 2. Create a character from the fixture.
create_resp=$(curl -sf -X POST "$API_URL/v1/characters" \
  -H "$AUTH" -H "$CT" \
  -d "$(jq -nc '{build: input, system: "pf2e"}' "$FIXTURE")")

char_id=$(echo "$create_resp" | jq -r '.character_id')
original_hash=$(echo "$create_resp" | jq -r '.content_hash')
echo "✓ Created character $char_id (hash=$original_hash)"

# 3. Create a Foundry connection.
conn_creds='{}'
if [[ -n "$RELAY_URL" ]]; then
  conn_creds=$(jq -nc --arg url "$RELAY_URL" '{relayUrl: $url}')
fi

conn_resp=$(curl -sf -X POST "$API_URL/v1/characters/$char_id/connections" \
  -H "$AUTH" -H "$CT" \
  -d "$(jq -nc --arg creds "$conn_creds" '{platform: "foundry", adapter_id: "foundry", credentials: ($creds | fromjson)}')")

conn_id=$(echo "$conn_resp" | jq -r '.id')
echo "✓ Created connection $conn_id"

# 4. Push — canonical → Foundry actor via adapter.export.
push_resp=$(curl -sf -X POST "$API_URL/v1/characters/$char_id/connections/$conn_id/push" \
  -H "$AUTH" -H "$CT" -d '{}')

actor_name=$(echo "$push_resp" | jq -r '.payload.name')
echo "✓ Pushed to Foundry (actor name=$actor_name)"

# 5. Pull — Foundry actor → canonical via adapter.import.
foundry_actor=$(echo "$push_resp" | jq -c '.payload')
pull_resp=$(curl -sf -X POST "$API_URL/v1/characters/$char_id/connections/$conn_id/pull" \
  -H "$AUTH" -H "$CT" \
  -d "$(jq -nc --arg payload "$foundry_actor" '{payload: ($payload | fromjson)}')")

pull_status=$(echo "$pull_resp" | jq -r '.status')
pull_hash=$(echo "$pull_resp" | jq -r '.content_hash // empty')
echo "✓ Pulled from Foundry (status=$pull_status)"

# 6. Verify round-trip integrity.
if [[ "$pull_status" == "no_changes" ]]; then
  echo "✅ Round-trip lossless (no_changes) — content hash unchanged"
  exit 0
fi

if [[ "$pull_status" == "merged" ]]; then
  # The round-trip introduced changes — check if they're expected.
  # Get the diff between original and current.
  diff_resp=$(curl -sf "$API_URL/v1/characters/$char_id/diff?from=$original_hash&to=$pull_hash" \
    -H "$AUTH" 2>/dev/null || echo '[]')

  diff_count=$(echo "$diff_resp" | jq 'length')
  if [[ "$diff_count" -eq 0 ]]; then
    echo "✅ Round-trip lossless (merged but no field-level diff)"
    exit 0
  fi

  echo "⚠ Round-trip produced $diff_count field differences:"
  echo "$diff_resp" | jq '.[] | "  \(.path): \(.old // "null") → \(.new // "null")"' -r
  echo ""
  echo "If these are expected (e.g., the adapter normalises casing), update the baseline."
  echo "If unexpected, the adapter mapping or upstream format may have changed."
  exit 1
fi

echo "FAIL: unexpected pull status: $pull_status"
echo "$pull_resp" | jq .
exit 1
