#!/usr/bin/env bash
# Refresh poster_url for MangaUpdates-sourced media items.
#
# Why: MangaUpdates CDN URLs are NOT immutable. When a series cover is
# replaced, the old cdn.mangaupdates.com/image/iNNNNNN.jpg file is deleted
# (404) while our DB keeps the snapshot captured at add time. Unlike TMDB
# (content-addressed paths), the stored URL eventually rots.
#
# What it does:
#   1. SELECT id, external_id, poster_url FROM media_items
#      WHERE provider = 'mangaupdates'
#   2. GET https://api.mangaupdates.com/v1/series/{external_id}
#   3. If image.url.original differs from the stored value -> UPDATE poster_url
#
# Idempotent: a second run changes nothing.
#
# Modes (auto-detected):
#   k8s    -> kubectl exec into statefulset/postgres (default)
#   direct -> psql against $PGHOST (e.g. a port-forward)
#
# Usage:
#   ./scripts/refresh-posters.sh                                # apply updates
#   DRY_RUN=1 ./scripts/refresh-posters.sh                      # report only
#   KUBECTL="sudo k3s kubectl" ./scripts/refresh-posters.sh     # VPS1 (no kubeconfig in PATH)
#   PGHOST=127.0.0.1 ./scripts/refresh-posters.sh               # direct DB
#
# Env:
#   DRY_RUN    1 = report changes without writing them   (default: 0)
#   NAMESPACE  k8s namespace                             (default: mediatracker)
#   KUBECTL    kubectl command (may be multi-word)       (default: kubectl)
#   PGUSER     database user                             (default: Kin)
#   PGDATABASE database name                             (default: tracker)
#
# Tip: run periodically (cron / after adding many manga) to keep posters fresh.

set -euo pipefail

NAMESPACE="${NAMESPACE:-mediatracker}"
KUBECTL="${KUBECTL:-kubectl}"
DRY_RUN="${DRY_RUN:-0}"
PGUSER="${PGUSER:-Kin}"
PGDATABASE="${PGDATABASE:-tracker}"

MU_API="https://api.mangaupdates.com/v1/series"

echo "============================================"
echo "  MangaUpdates Poster Refresh"
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

# --- DB helper ---
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

echo ">> Checking connectivity to postgres..."
if ! psql_cmd "SELECT 1" | grep -q '^1$'; then
    echo "ERROR: cannot reach postgres." >&2
    echo "  k8s mode:    make sure kubectl works" >&2
    echo "               (on VPS1: KUBECTL=\"sudo k3s kubectl\")" >&2
    echo "  direct mode: set PGHOST (and PGPASSWORD) to a reachable database" >&2
    exit 1
fi
echo ">> Postgres: OK"
echo ""

# --- Fetch rows ---
rows=$(psql_cmd "
    SELECT id, external_id, COALESCE(poster_url, '')
    FROM media_items
    WHERE provider = 'mangaupdates'
    ORDER BY external_id
" | sed '/^$/d')

if [[ -z "$rows" ]]; then
    echo "No mangaupdates items found."
    exit 0
fi

total=$(echo "$rows" | wc -l)
echo "Items to check: $total"
echo ""

updated=0
unchanged=0
skip=0
fail=0
idx=0

while IFS='|' read -r mid eid current; do
    idx=$(( idx + 1 ))
    mid="${mid//[[:space:]]/}"
    eid="${eid//[[:space:]]/}"

    if ! resp=$(curl -sf --max-time 15 "$MU_API/$eid" 2>&1); then
        echo "  [$idx/$total] $eid — curl error: $resp"
        fail=$(( fail + 1 ))
        continue
    fi

    # Be nice to the MangaUpdates API (it rate-limits)
    sleep 0.5

    new_url=$(echo "$resp" | jq -r '.image.url.original // empty')

    if [[ -z "$new_url" ]]; then
        echo "  [$idx/$total] $eid — no image in API response, skip"
        skip=$(( skip + 1 ))
        continue
    fi

    if [[ "$new_url" == "$current" ]]; then
        echo "  [$idx/$total] $eid — unchanged"
        unchanged=$(( unchanged + 1 ))
        continue
    fi

    # Defense in depth: the URL goes into a single-quoted SQL literal
    if [[ "$new_url" == *"'"* ]]; then
        echo "  [$idx/$total] $eid — unexpected quote in URL, skip"
        skip=$(( skip + 1 ))
        continue
    fi

    if [[ "$DRY_RUN" == "1" ]]; then
        echo "  [$idx/$total] $eid — [dry-run] would update: ${current:-<empty>} -> $new_url"
        updated=$(( updated + 1 ))
        continue
    fi

    if ! out=$(psql_cmd "UPDATE media_items SET poster_url = '$new_url'
                         WHERE id = '$mid' AND provider = 'mangaupdates'" 2>&1); then
        echo "  [$idx/$total] $eid — UPDATE failed: $out"
        fail=$(( fail + 1 ))
        continue
    fi

    echo "  [$idx/$total] $eid — updated: ${current:-<empty>} -> $new_url"
    updated=$(( updated + 1 ))
done <<< "$rows"

echo ""
echo "============================================"
if [[ "$DRY_RUN" == "1" ]]; then
    echo "  Done (dry-run): $updated would update, $unchanged unchanged, $skip skipped, $fail failed"
else
    echo "  Done: $updated updated, $unchanged unchanged, $skip skipped, $fail failed"
fi
echo "============================================"
exit $(( fail > 0 ? 1 : 0 ))
