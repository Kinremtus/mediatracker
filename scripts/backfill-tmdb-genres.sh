#!/usr/bin/env bash
# Backfill genres for TMDB-sourced media_items with an empty genres array.
#
# Why: TMDB items added before `map_details` populated `genres` (or through a
# cached/search path) can have `genres = '{}'`. The drawer then shows no genre
# chips. This re-reads /movie/{id} or /tv/{id} and fills ONLY rows whose
# genres are still empty (never overwrites existing values).
#
# Modes (auto-detected):
#   k8s    -> kubectl exec into statefulset/postgres (default)
#   direct -> psql against $PGHOST (e.g. a port-forward)
#
# Usage:
#   ./scripts/backfill-tmdb-genres.sh                             # apply
#   DRY_RUN=1 ./scripts/backfill-tmdb-genres.sh                   # report only
#   KUBECTL="sudo k3s kubectl" ./scripts/backfill-tmdb-genres.sh  # VPS1
#   PGHOST=127.0.0.1 ./scripts/backfill-tmdb-genres.sh            # direct DB
#
# Env:
#   TMDB_API_KEY  TMDB v3 key (else read from K8s secret app-secret)
#   DRY_RUN       1 = report only                        (default: 0)
#   NAMESPACE     k8s namespace                          (default: mediatracker)
#   KUBECTL       kubectl command (may be multi-word)    (default: kubectl)
#   PGUSER        db user                                (default: Kin)
#   PGDATABASE    db name                                (default: tracker)

set -euo pipefail

NAMESPACE="${NAMESPACE:-mediatracker}"
KUBECTL="${KUBECTL:-kubectl}"
DRY_RUN="${DRY_RUN:-0}"
PGUSER="${PGUSER:-Kin}"
PGDATABASE="${PGDATABASE:-tracker}"

echo "============================================"
echo "  TMDB Genres Backfill"
echo "============================================"
if [[ "$DRY_RUN" == "1" ]]; then
    echo "  DRY RUN — no changes will be written"
fi

# --- Mode detection ---
if [[ -n "${PGHOST:-}" ]]; then
    MODE="direct"
else
    MODE="k8s"
fi
echo ">> Mode: $MODE (namespace=$NAMESPACE, db=$PGDATABASE)"

# --- DB helper (same pattern as scripts/refresh-posters.sh) ---
psql_cmd() {
    if [[ "$MODE" == "k8s" ]]; then
        # $KUBECTL is intentionally unquoted: it may be "sudo k3s kubectl"
        # shellcheck disable=SC2086
        $KUBECTL exec statefulset/postgres -n "$NAMESPACE" -- \
            psql -U "$PGUSER" -d "$PGDATABASE" -t -A -F'|' -c "$1"
    else
        psql -h "$PGHOST" -p "${PGPORT:-5432}" -U "$PGUSER" -d "$PGDATABASE" \
            -t -A -F'|' -c "$1"
    fi
}

# --- API key ---
if [[ -z "${TMDB_API_KEY:-}" && "$MODE" == "k8s" ]]; then
    echo ">> Reading TMDB_API_KEY from K8s secret..."
    TMDB_API_KEY=$($KUBECTL get secret app-secret -n "$NAMESPACE" \
        -o jsonpath='{.data.TMDB_API_KEY}' | base64 -d | xargs)
fi
if [[ -z "${TMDB_API_KEY:-}" ]]; then
    echo "ERROR: TMDB_API_KEY not set (env or K8s secret app-secret)" >&2
    exit 1
fi
echo ">> TMDB_API_KEY: OK"

echo ">> Checking connectivity to postgres..."
if ! psql_cmd "SELECT 1" | grep -q '^1$'; then
    echo "ERROR: cannot reach postgres." >&2
    echo "  k8s mode:    on VPS1 use KUBECTL=\"sudo k3s kubectl\"" >&2
    echo "  direct mode: set PGHOST (and PGPASSWORD)" >&2
    exit 1
fi
echo ">> Postgres: OK"
echo ""

rows=$(psql_cmd "
    SELECT id, external_id, media_type
    FROM media_items
    WHERE provider = 'tmdb'
      AND cardinality(genres) = 0
    ORDER BY external_id
" | sed '/^$/d')

if [[ -z "$rows" ]]; then
    echo "No TMDB items with empty genres."
    exit 0
fi

total=$(echo "$rows" | wc -l)
echo "Items to check: $total"
echo ""

updated=0
skip=0
fail=0
idx=0

while IFS='|' read -r mid eid mtype; do
    idx=$(( idx + 1 ))
    mid="${mid//[[:space:]]/}"
    eid="${eid//[[:space:]]/}"
    mtype="${mtype//[[:space:]]/}"

    case "$mtype" in
        movie|animated-movies)  tmdb_type="movie" ;;
        series|dramas|cartoons) tmdb_type="tv"    ;;
        *)
            echo "  [$idx/$total] [$mtype] $eid — unknown media_type, skip"
            skip=$(( skip + 1 ))
            continue
            ;;
    esac

    api_url="https://api.themoviedb.org/3/$tmdb_type/$eid?api_key=$TMDB_API_KEY&language=ru-RU"
    if ! resp=$(curl -sf --max-time 15 "$api_url" 2>&1); then
        echo "  [$idx/$total] [$mtype] $eid — curl error: $resp"
        fail=$(( fail + 1 ))
        continue
    fi

    genres_json=$(echo "$resp" | jq -c '[.genres[].name]') || genres_json=""
    if [[ -z "$genres_json" || "$genres_json" == "[]" ]]; then
        echo "  [$idx/$total] [$mtype] $eid — TMDB returned no genres, skip"
        skip=$(( skip + 1 ))
        continue
    fi

    if [[ "$DRY_RUN" == "1" ]]; then
        echo "  [$idx/$total] [$mtype] $eid — [dry-run] would set genres = $genres_json"
        updated=$(( updated + 1 ))
        continue
    fi

    # genres_json comes from JSON (double quotes); escape single quotes
    # defensively before embedding in a SQL string literal.
    sql_json="${genres_json//\'/\'\'}"
    if ! out=$(psql_cmd "UPDATE media_items
                         SET genres = ARRAY(SELECT jsonb_array_elements_text('$sql_json'::jsonb)),
                             updated_at = now()
                         WHERE id = '$mid'
                           AND provider = 'tmdb'
                           AND cardinality(genres) = 0" 2>&1); then
        echo "  [$idx/$total] [$mtype] $eid — UPDATE failed: $out"
        fail=$(( fail + 1 ))
        continue
    fi

    echo "  [$idx/$total] [$mtype] $eid — OK: $genres_json"
    updated=$(( updated + 1 ))

    sleep 0.05
done <<< "$rows"

echo ""
echo "============================================"
if [[ "$DRY_RUN" == "1" ]]; then
    echo "  Done (dry-run): $updated would update, $skip skipped, $fail failed"
else
    echo "  Done: $updated updated, $skip skipped, $fail failed"
fi
echo "============================================"
exit $(( fail > 0 ? 1 : 0 ))
