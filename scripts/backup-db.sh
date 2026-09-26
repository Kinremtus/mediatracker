#!/usr/bin/env bash
#
# Logical PostgreSQL backup (custom format `-Fc`), k3s-aware.
#
# Modes (auto-detected, override with USE_K8S=1 or KUBE=1):
#   k8s    -> kubectl exec into the `postgres` StatefulSet and run pg_dump there
#   direct -> run pg_dump against $PGHOST:$PGPORT (e.g. a port-forward)
# If neither a kube context nor PGHOST is available the script prints usage.
#
# Connection settings come from the environment (in k3s the `db-secret`
# already exports PGUSER/PGDATABASE/PGHOST/PGPORT/PGPASSWORD -- the same keys
# libpq itself understands, so no credentials are hardcoded here).
#
# Optional env:
#   BACKUP_DIR       output directory                     (default: ./backups)
#   RETENTION_DAYS   prune local dumps older than N days  (default: 7)
#   NAMESPACE        k8s namespace                        (default: mediatracker)
#   AGE_PUBLIC_KEY   age recipient; when set the dump is encrypted (.dump.age)
#   BACKUP_TZ        timezone used in the filename         (default: Europe/Minsk)
#   R2_BUCKET        offsite bucket; when set, the dump is also copied with
#                    rclone using the remote below (offline: warn only)
#   R2_REMOTE        rclone remote name                    (default: r2)
#   BACKUP_REQUIRE_OFFSITE  1 = fail when the offsite copy fails (default: 0)
#
# Examples:
#   PGHOST=127.0.0.1 PGUSER=Kin PGDATABASE=tracker ./scripts/backup-db.sh
#   USE_K8S=1 AGE_PUBLIC_KEY=age1... ./scripts/backup-db.sh
#
set -euo pipefail

BACKUP_DIR="${BACKUP_DIR:-./backups}"
RETENTION_DAYS="${RETENTION_DAYS:-7}"
NAMESPACE="${NAMESPACE:-mediatracker}"
STSET="statefulset/postgres"
# The dump is produced inside the postgres pod itself, so the service DNS name
# is the default only for documentation; kubectl exec ignores PGHOST.
PGUSER="${PGUSER:-${POSTGRES_USER:-}}"
PGDATABASE="${PGDATABASE:-${POSTGRES_DB:-}}"

log() { printf '[%s] %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$*"; }
die() { log "ERROR: $*" >&2; exit 1; }

have_cmd() { command -v "$1" >/dev/null 2>&1; }

detect_mode() {
    if [[ "${USE_K8S:-0}" == "1" || -n "${KUBE:-}" ]]; then
        echo "k8s"; return
    fi
    if have_cmd kubectl && kubectl config current-context >/dev/null 2>&1; then
        echo "k8s"; return
    fi
    if [[ -n "${PGHOST:-}" ]]; then
        echo "direct"; return
    fi
    echo ""
}

MODE="$(detect_mode)"
if [[ -z "$MODE" ]]; then
    cat >&2 <<'USAGE'
No backup target found.

  * For k3s: run where kubectl has a context for the cluster,
    or force it with USE_K8S=1.
  * For a direct database: export PGHOST (and optionally PGPORT).

Also required (usually provided by the `db-secret` secret):
  PGUSER, PGDATABASE  (PGPASSWORD is read by libpq automatically)
USAGE
    exit 1
fi

[[ -n "$PGUSER" ]]     || die "PGUSER is not set (export it or use the db-secret)"
[[ -n "$PGDATABASE" ]] || die "PGDATABASE is not set (export it or use the db-secret)"

if [[ -n "${AGE_PUBLIC_KEY:-}" ]]; then
    have_cmd age || die "AGE_PUBLIC_KEY is set but the 'age' binary was not found"
fi

mkdir -p "$BACKUP_DIR"

TIMESTAMP="$(TZ="${BACKUP_TZ:-Europe/Minsk}" date '+%Y%m%d_%H%M%S')"
if [[ -n "${AGE_PUBLIC_KEY:-}" ]]; then
    BACKUP_FILE="${BACKUP_DIR}/${TIMESTAMP}.dump.age"
else
    BACKUP_FILE="${BACKUP_DIR}/${TIMESTAMP}.dump"
fi
TMP_FILE="${BACKUP_FILE}.partial"

cleanup() { [[ -n "${TMP_FILE:-}" && -f "${TMP_FILE:-}" ]] && rm -f "$TMP_FILE" || true; }
trap cleanup EXIT

run_dump() {
    case "$MODE" in
        k8s)
            kubectl exec -n "$NAMESPACE" "$STSET" -- \
                pg_dump -Fc -U "$PGUSER" -d "$PGDATABASE"
            ;;
        direct)
            pg_dump -Fc -h "$PGHOST" -p "${PGPORT:-5432}" -U "$PGUSER" -d "$PGDATABASE"
            ;;
        *)
            die "internal error: unknown mode '$MODE'"
            ;;
    esac
}

log "Backup mode: $MODE (namespace=$NAMESPACE, db=$PGDATABASE)"

if [[ -n "${AGE_PUBLIC_KEY:-}" ]]; then
    log "Encrypting with age recipient"
    run_dump | age -r "$AGE_PUBLIC_KEY" > "$TMP_FILE"
else
    log "No AGE_PUBLIC_KEY set - writing unencrypted dump"
    run_dump > "$TMP_FILE"
fi

mv -f "$TMP_FILE" "$BACKUP_FILE"
TMP_FILE=""

SIZE="$(du -h "$BACKUP_FILE" | cut -f1)"
if have_cmd sha256sum; then
    CHECKSUM="$(sha256sum "$BACKUP_FILE" | cut -d' ' -f1)"
    log "Backup saved: $BACKUP_FILE ($SIZE, sha256=$CHECKSUM)"
else
    log "Backup saved: $BACKUP_FILE ($SIZE)"
fi

# Optional offsite copy (mirrors the k8s CronJob behaviour). The rclone remote
# must exist in the local rclone.conf, e.g.:
#   rclone config create r2 s3 provider Cloudflare \
#     access_key_id <key> secret_access_key <secret> \
#     endpoint https://<account-id>.r2.cloudflarestorage.com
if [[ -n "${R2_BUCKET:-}" ]]; then
    if have_cmd rclone; then
        log "Uploading to ${R2_REMOTE:-r2}:${R2_BUCKET}"
        if rclone copy "$BACKUP_FILE" "${R2_REMOTE:-r2}:${R2_BUCKET}"; then
            log "Offsite copy OK -> ${R2_REMOTE:-r2}:${R2_BUCKET}"
        elif [[ "${BACKUP_REQUIRE_OFFSITE:-0}" == "1" ]]; then
            die "offsite upload failed and BACKUP_REQUIRE_OFFSITE=1"
        else
            log "WARNING: offsite upload failed; local dump is still valid"
        fi
    elif [[ "${BACKUP_REQUIRE_OFFSITE:-0}" == "1" ]]; then
        die "R2_BUCKET is set but rclone is not installed"
    else
        log "WARNING: R2_BUCKET set but rclone is missing - skipping offsite copy"
    fi
fi

# Retention: local custom-format dumps, encrypted or not.
log "Pruning local backups older than ${RETENTION_DAYS} days in $BACKUP_DIR"
find "$BACKUP_DIR" -maxdepth 1 -type f \( -name '*.dump' -o -name '*.dump.age' \) \
    -mtime +"$RETENTION_DAYS" -print -delete || true

log "Done."
