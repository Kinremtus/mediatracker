#!/bin/bash
# Backfill media details for all providers via /admin/refresh-details.
#
# What it does:
#   For each provider in PROVIDERS, sends POST /admin/refresh-details
#   with provider=<name> limit=500. Server fetches fresh data from the
#   external API and updates the media_items row. Existing tracking
#   entries are not affected.
#
# How to run:
#   1. Open your browser, log in to MediaTracker.
#   2. Open DevTools (F12) -> Application -> Cookies -> copy the
#      value of the 'session_id' cookie.
#   3. From the project root on the SERVER (where docker compose runs):
#        ./scripts/backfill-details.sh
#   4. Paste the cookie when prompted.
#
# Or run non-interactively:
#   SESSION_COOKIE="your-cookie-value" ./scripts/backfill-details.sh [provider1 provider2 ...]
#
# The script defaults to running all providers in sequence. Pass
# provider names as arguments to limit the run, e.g.:
#   ./scripts/backfill-details.sh mangaupdates mal shikimori

set -euo pipefail

BASE_URL="${MEDIATRACKER_URL:-http://localhost:8080}"
LIMIT="${BACKFILL_LIMIT:-500}"

PROVIDERS=(
    mangaupdates
    mal
    shikimori
    tmdb
    rawg
    igdb
    google_books
    openlibrary
)

if [[ $# -gt 0 ]]; then
    PROVIDERS=("$@")
fi

if [[ -z "${SESSION_COOKIE:-}" ]]; then
    echo "============================================================"
    echo "  MediaTracker — Backfill details for all providers"
    echo "============================================================"
    echo ""
    echo "Need your session cookie to authenticate as admin."
    echo ""
    echo "How to get it:"
    echo "  1. Open ${BASE_URL} in your browser and log in"
    echo "  2. Press F12 to open DevTools"
    echo "  3. Go to Application -> Cookies -> ${BASE_URL}"
    echo "  4. Copy the 'Value' of the 'session_id' cookie"
    echo ""
    read -r -p "Paste session_id value: " SESSION_COOKIE
    echo ""
fi

if [[ -z "${SESSION_COOKIE}" ]]; then
    echo "ERROR: no session cookie provided, aborting." >&2
    exit 1
fi

echo "Using URL:    ${BASE_URL}"
echo "Using limit:  ${LIMIT} items per provider"
echo "Providers:    ${PROVIDERS[*]}"
echo ""

for provider in "${PROVIDERS[@]}"; do
    echo "------------------------------------------------------------"
    echo ">> Refreshing: ${provider}"
    echo "------------------------------------------------------------"

    http_code=$(curl -sS -o "/tmp/backfill-${provider}.html" -w "%{http_code}" \
        -X POST "${BASE_URL}/admin/refresh-details" \
        -H "Cookie: session_id=${SESSION_COOKIE}" \
        -H "Content-Type: application/x-www-form-urlencoded" \
        --data-urlencode "provider=${provider}" \
        --data-urlencode "limit=${LIMIT}" \
        --max-time 900) || {
            echo "  FAILED: curl error for ${provider}" >&2
            continue
        }

    if [[ "${http_code}" == "302" ]] || [[ "${http_code}" == "303" ]]; then
        echo "  REDIRECT (status ${http_code}) — probably not logged in or not admin."
        echo "  Check your session cookie."
        continue
    fi

    if [[ "${http_code}" != "200" ]]; then
        echo "  HTTP ${http_code} — see /tmp/backfill-${provider}.html"
        continue
    fi

    summary=$(grep -oE "Обновлено:[^<]*|Refreshed:[^<]*|обновл[её]нн?[ыо]?[^<]*[0-9]+[^<]*" \
        "/tmp/backfill-${provider}.html" | head -1 || echo "")

    if [[ -n "${summary}" ]]; then
        echo "  ${summary}"
    else
        echo "  Done (no summary parsed). Check /tmp/backfill-${provider}.html"
    fi
    echo ""
done

echo "============================================================"
echo "  Backfill finished"
echo "============================================================"
echo "Tip: tail server logs to see per-item details:"
echo "  docker compose logs --tail=200 app | grep -i refresh"
