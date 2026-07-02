#!/bin/bash
# Straight-to-DB backfill of TMDB metadata (episodes, runtime_minutes).
#
# Reads TMDB_API_KEY from K8s secret, queries Postgres via kubectl exec,
# calls TMDB API directly, and updates NULL fields.
#
# Usage:
#   ./scripts/backfill-tmdb-metadata.sh                    # reads API key from K8s
#   TMDB_API_KEY=xxx ./scripts/backfill-tmdb-metadata.sh   # explicit key
#
# Environment:
#   NAMESPACE  — K8s namespace (default: mediatracker)

set -euo pipefail

NAMESPACE="${NAMESPACE:-mediatracker}"

echo "============================================"
echo "  TMDB Metadata Backfill"
echo "============================================"

# --- API key (trim whitespace — base64 often leaves trailing \n) ---
TMDB_API_KEY="${TMDB_API_KEY:-}"
if [[ -z "$TMDB_API_KEY" ]]; then
    echo ">> Reading TMDB_API_KEY from K8s secret..."
    TMDB_API_KEY=$(kubectl get secret app-secret -n "$NAMESPACE" \
        -o jsonpath='{.data.TMDB_API_KEY}' | base64 -d | xargs)
fi

if [[ -z "$TMDB_API_KEY" ]]; then
    echo "ERROR: TMDB_API_KEY not found (set via env or K8s secret)" >&2
    exit 1
fi
echo ">> TMDB_API_KEY: OK (${#TMDB_API_KEY} chars, no whitespace)"

# --- DB helper ---
psql_cmd() {
    kubectl exec statefulset/postgres -n "$NAMESPACE" -- \
        psql -U Kin -d tracker -t -A -F'|' -c "$1"
}

echo ">> Checking connectivity to postgres..."
if ! psql_cmd "SELECT 1" | grep -q '^1$'; then
    echo "ERROR: Cannot reach postgres" >&2
    exit 1
fi
echo ">> Postgres: OK"

echo ""

# --- Fetch items needing backfill ---
items=$(psql_cmd "
    SELECT external_id, media_type
    FROM media_items
    WHERE provider = 'tmdb'
      AND (episodes IS NULL OR runtime_minutes IS NULL)
    ORDER BY external_id
" | sed '/^$/d')

if [[ -z "$items" ]]; then
    echo "No items need backfill."
    exit 0
fi

total=$(echo "$items" | wc -l)
echo "Items to backfill: $total"
echo ""

ok=0
skip=0
fail=0
idx=0

while IFS='|' read -r eid mtype; do
    ((idx++))
    eid="${eid//[[:space:]]/}"
    mtype="${mtype//[[:space:]]/}"

    # Map internal media_type to TMDB endpoint
    tmdb_type=""
    case "$mtype" in
        movie|animated-movies)   tmdb_type="movie" ;;
        series|dramas|cartoons)  tmdb_type="tv"   ;;
        *)
            echo "  [$idx/$total] [$mtype] $eid — unknown media_type, skip"
            ((skip++))
            continue
            ;;
    esac

    # Call TMDB API (capture stderr too, so we see curl errors)
    api_url="https://api.themoviedb.org/3/$tmdb_type/$eid?api_key=$TMDB_API_KEY&language=ru-RU"
    if ! resp=$(curl -sf "$api_url" 2>&1); then
        echo "  [$idx/$total] [$mtype] $eid — curl error: $resp"
        ((fail++))
        continue
    fi

    episodes=$(echo "$resp" | jq -r '.number_of_episodes // empty')
    runtime=$(echo "$resp" | jq -r '.runtime // empty')

    # Build SET clause
    sets=()
    [[ -n "$episodes" ]] && sets+=("episodes = $episodes")
    [[ -n "$runtime"  ]] && sets+=("runtime_minutes = $runtime")

    if [[ ${#sets[@]} -eq 0 ]]; then
        echo "  [$idx/$total] [$mtype] $eid — nothing to update (no episodes, no runtime in API response)"
        ((skip++))
        continue
    fi

    # Join with commas
    set_sql=""
    sep=""
    for s in "${sets[@]}"; do
        set_sql="${set_sql}${sep}${s}"
        sep=", "
    done

    if ! out=$(psql_cmd "UPDATE media_items SET $set_sql WHERE provider='tmdb' AND external_id='$eid'" 2>&1); then
        echo "  [$idx/$total] [$mtype] $eid — UPDATE failed: $out"
        ((fail++))
        continue
    fi

    echo "  [$idx/$total] [$mtype] $eid — OK: ${set_sql}"
    ((ok++))

    # Be nice to TMDB API
    sleep 0.05
done <<< "$items"

echo ""
echo "============================================"
echo "  Done: $ok updated, $skip skipped, $fail failed"
echo "============================================"
exit $(( fail > 0 ? 1 : 0 ))
