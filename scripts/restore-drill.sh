#!/usr/bin/env bash
#
# Restore drill -- LOCAL MACHINE ONLY.
#
# Spins up an ephemeral PostgreSQL in Docker, restores the newest backup from
# ./backups into it, runs control queries, and tears the container down. This
# proves a dump is actually restorable without touching the real database.
#
# It requires a working Docker daemon and is never meant to run in-cluster.
#
# Optional env:
#   BACKUP_DIR      directory holding *.dump / *.dump.age   (default: ./backups)
#   DRILL_IMAGE     postgres image                          (default: postgres:17-alpine)
#   DRILL_PORT      host port mapped to 5432                (default: 55432)
#   DRILL_CONTAINER container name                          (default: mt-drill)
#   AGE_IDENTITY_FILE  required when the newest dump is *.age
#
set -euo pipefail

BACKUP_DIR="${BACKUP_DIR:-./backups}"
DRILL_IMAGE="${DRILL_IMAGE:-postgres:17-alpine}"
DRILL_PORT="${DRILL_PORT:-55432}"
DRILL_CONTAINER="${DRILL_CONTAINER:-mt-drill}"
DRILL_PASSWORD="${DRILL_PASSWORD:-drill}"
DRILL_DB="${DRILL_DB:-tracker}"
DRILL_USER="${DRILL_USER:-postgres}"

log() { printf '[drill] %s\n' "$*"; }
die() { log "ERROR: $*" >&2; exit 1; }

have_cmd() { command -v "$1" >/dev/null 2>&1; }

have_cmd docker || die "docker is required (this drill is local-machine only)"
docker info >/dev/null 2>&1 || die "the Docker daemon is not reachable"

# Newest dump first; globs are intentionally unquoted so they expand.
LATEST=""
for f in "$BACKUP_DIR"/*.dump "$BACKUP_DIR"/*.dump.age; do
    [[ -e "$f" ]] || continue
    if [[ -z "$LATEST" || "$f" -nt "$LATEST" ]]; then
        LATEST="$f"
    fi
done
[[ -n "$LATEST" ]] || die "no *.dump / *.dump.age found in $BACKUP_DIR"
log "Latest backup: $LATEST"

cleanup() {
    log "Removing drill container '$DRILL_CONTAINER'"
    docker rm -f "$DRILL_CONTAINER" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

# A leftover container from an interrupted run must not block us.
docker rm -f "$DRILL_CONTAINER" >/dev/null 2>&1 || true

log "Starting ephemeral Postgres ($DRILL_IMAGE) on 127.0.0.1:$DRILL_PORT"
docker run --rm -d --name "$DRILL_CONTAINER" \
    -e "POSTGRES_PASSWORD=$DRILL_PASSWORD" \
    -e "POSTGRES_DB=$DRILL_DB" \
    -p "127.0.0.1:${DRILL_PORT}:5432" \
    "$DRILL_IMAGE" >/dev/null

log "Waiting for Postgres to accept connections ..."
READY=0
for _ in $(seq 1 60); do
    if docker exec "$DRILL_CONTAINER" \
        pg_isready -U "$DRILL_USER" -d "$DRILL_DB" >/dev/null 2>&1; then
        READY=1
        break
    fi
    sleep 1
done
[[ "$READY" == "1" ]] || die "Postgres did not become ready within 60s"

EFFECTIVE="$LATEST"
ENCRYPTED=0
if [[ "$EFFECTIVE" == *.age ]]; then
    ENCRYPTED=1
    EFFECTIVE="${EFFECTIVE%.age}"
fi

open_stream() {
    if [[ "$ENCRYPTED" == "1" ]]; then
        have_cmd age || die "'age' is required to decrypt $LATEST"
        [[ -n "${AGE_IDENTITY_FILE:-}" ]] || die "AGE_IDENTITY_FILE is required for $LATEST"
        [[ -f "$AGE_IDENTITY_FILE" ]] || die "AGE_IDENTITY_FILE not found: $AGE_IDENTITY_FILE"
        age -d -i "$AGE_IDENTITY_FILE" "$LATEST"
    else
        cat -- "$LATEST"
    fi
}

log "Restoring into the drill database ..."
case "$EFFECTIVE" in
    *.sql.gz)
        open_stream | gunzip -c | docker exec -i "$DRILL_CONTAINER" \
            psql -v ON_ERROR_STOP=1 -U "$DRILL_USER" -d "$DRILL_DB"
        ;;
    *.dump|*.backup)
        open_stream | docker exec -i "$DRILL_CONTAINER" \
            pg_restore --clean --if-exists --no-owner -U "$DRILL_USER" -d "$DRILL_DB"
        ;;
    *.sql)
        open_stream | docker exec -i "$DRILL_CONTAINER" \
            psql -v ON_ERROR_STOP=1 -U "$DRILL_USER" -d "$DRILL_DB"
        ;;
    *)
        die "unsupported backup format: $LATEST"
        ;;
esac

log "Control queries:"
docker exec "$DRILL_CONTAINER" psql -v ON_ERROR_STOP=1 -U "$DRILL_USER" -d "$DRILL_DB" \
    -c "SELECT count(*) AS users FROM users;" \
    -c "SELECT count(*) AS tracking_entries FROM tracking_entries;"

USERS="$(docker exec "$DRILL_CONTAINER" psql -tAc \
    "SELECT count(*) FROM users;" -U "$DRILL_USER" -d "$DRILL_DB" | tr -d '[:space:]')"

if [[ -z "$USERS" || ! "$USERS" =~ ^[0-9]+$ || "$USERS" -eq 0 ]]; then
    die "drill FAILED: users count is '$USERS'"
fi

log "Drill PASSED: users=$USERS"
