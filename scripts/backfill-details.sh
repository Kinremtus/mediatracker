#!/bin/bash
# Backfill media details for all providers via /admin/refresh-details.
#
# What it does:
#   For each provider in PROVIDERS, sends POST /admin/refresh-details
#   with provider=<name> limit=500 (or BACKFILL_LIMIT). Server fetches
#   fresh data from the external API and updates the media_items row.
#   Existing tracking entries are not affected.
#
# How to run — three modes:
#
#   Mode 1: Browser (recommended, safest):
#     Just go to /admin in your browser, select provider, click refresh.
#     Limit=50 per run to fit Cloudflare Tunnel timeout (~100s).
#
#   Mode 2: kubectl port-forward (k3s, this script):
#     ./scripts/backfill-details.sh [provider1 provider2 ...]
#
#   Mode 3: Direct URL (docker compose, nginx, etc.):
#     MEDIATRACKER_URL="http://localhost:8080" ./scripts/backfill-details.sh
#
# How to get a session cookie for Mode 2/3:
#   1. Open MediaTracker in your browser and log in.
#   2. Open DevTools (F12) -> Application -> Cookies.
#   3. Copy the value of the 'session_id' cookie.
#   4. Paste when prompted, or pass via SESSION_COOKIE env var.
#
# Non-interactive:
#   SESSION_COOKIE="your-cookie" ./scripts/backfill-details.sh tmdb rawg

set -euo pipefail

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

# ── Detect mode ──────────────────────────────────────────────

USE_PORT_FORWARD=false
BASE_URL="${MEDIATRACKER_URL:-}"

if [[ -z "${BASE_URL}" ]] && command -v kubectl &>/dev/null; then
    USE_PORT_FORWARD=true
    BASE_URL="http://localhost:8080"
elif [[ -z "${BASE_URL}" ]]; then
    echo "ERROR: no MEDIATRACKER_URL set and kubectl not found." >&2
    echo "  Either set MEDIATRACKER_URL to your app's URL, or" >&2
    echo "  install kubectl and configure it for your k3s cluster." >&2
    exit 1
fi

# ── Port-forward setup ───────────────────────────────────────

PF_PID=""
if $USE_PORT_FORWARD; then
    KUBECTL_EXTRA=""
    if [[ -f /etc/rancher/k3s/k3s.yaml ]]; then
        KUBECTL_EXTRA="--kubeconfig /etc/rancher/k3s/k3s.yaml"
    fi

    echo ">> Starting kubectl port-forward to mediatracker app..."
    kubectl $KUBECTL_EXTRA port-forward -n mediatracker deployment/app 8080:8080 &
    PF_PID=$!

    # Kill port-forward on script exit (any signal)
    trap 'echo ">> Cleaning up port-forward..."; kill $PF_PID 2>/dev/null; wait $PF_PID 2>/dev/null' EXIT INT TERM

    # Wait for tunnel to be ready
    sleep 2
    if ! kill -0 $PF_PID 2>/dev/null; then
        echo "ERROR: port-forward failed to start." >&2
        exit 1
    fi
fi

# ── Session cookie ───────────────────────────────────────────

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

# ── Backfill loop ────────────────────────────────────────────

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
echo "Tip: tail app logs on VPS1:"
echo "  kubectl logs -n mediatracker deployment/app --tail=200 | grep -i refresh"
