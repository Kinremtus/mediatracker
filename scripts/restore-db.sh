#!/usr/bin/env bash
#
# Logical PostgreSQL restore, k3s-aware. Supports:
#   *.dump / *.backup   custom-format archive  -> pg_restore (--clean --if-exists)
#   *.dump.age          age-encrypted archive  -> decrypt then pg_restore
#   *.sql.gz            gzipped SQL            -> gunzip | psql
#   *.sql               plain SQL              -> psql
#
# Target detection mirrors backup-db.sh:
#   k8s    -> kubectl exec -i into the `postgres` StatefulSet (streams the dump)
#   direct -> connect to $PGHOST:$PGPORT
#
# Connection settings come from the environment (in k3s the `db-secret`
# already exports PGUSER/PGDATABASE/PGHOST/PGPORT/PGPASSWORD).
#
# Safety: the restore overwrites database objects, so it requires an explicit
# confirmation. Non-interactive callers must pass CONFIRM=yes.
#
# Optional env:
#   CONFIRM=yes        skip the interactive prompt (required when stdin is not a tty)
#   AGE_IDENTITY_FILE  age private key, required for *.age dumps
#   NAMESPACE          k8s namespace (default: mediatracker)
#   USE_K8S=1 / KUBE   force k8s mode
#
# Examples:
#   CONFIRM=yes AGE_IDENTITY_FILE=~/.age/key.txt ./scripts/restore-db.sh backups/20260101_030000.dump.age
#   USE_K8S=1 CONFIRM=yes ./scripts/restore-db.sh backups/20260101_030000.dump
#
set -euo pipefail

NAMESPACE="${NAMESPACE:-mediatracker}"
STSET="statefulset/postgres"
PGUSER="${PGUSER:-${POSTGRES_USER:-}}"
PGDATABASE="${PGDATABASE:-${POSTGRES_DB:-}}"

log() { printf '[%s] %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$*"; }
die() { log "ERROR: $*" >&2; exit 1; }

have_cmd() { command -v "$1" >/dev/null 2>&1; }

BACKUP_FILE="${1:-}"
if [[ -z "$BACKUP_FILE" ]]; then
    cat >&2 <<'USAGE'
Usage: restore-db.sh <backup_file>

Supported: *.dump, *.backup, *.dump.age, *.sql.gz, *.sql.gz.age, *.sql

Available backups:
USAGE
    ls -lh ./backups/*.dump ./backups/*.dump.age ./backups/*.sql.gz 2>/dev/null || echo "  (none found)"
    exit 1
fi

[[ -f "$BACKUP_FILE" ]] || die "file not found: $BACKUP_FILE"

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
[[ -n "$MODE" ]] || die "no restore target: set PGHOST (direct) or USE_K8S=1 / provide a kube context"
[[ -n "$PGUSER" ]]     || die "PGUSER is not set"
[[ -n "$PGDATABASE" ]] || die "PGDATABASE is not set"

# Strip an outer .age suffix to classify the underlying dump format.
EFFECTIVE="$BACKUP_FILE"
ENCRYPTED=0
if [[ "$EFFECTIVE" == *.age ]]; then
    ENCRYPTED=1
    EFFECTIVE="${EFFECTIVE%.age}"
fi

if [[ "$ENCRYPTED" == "1" ]]; then
    [[ -n "${AGE_IDENTITY_FILE:-}" ]] || die "AGE_IDENTITY_FILE is required to decrypt $BACKUP_FILE"
    [[ -f "$AGE_IDENTITY_FILE" ]] || die "AGE_IDENTITY_FILE not found: $AGE_IDENTITY_FILE"
    have_cmd age || die "the 'age' binary is required to decrypt $BACKUP_FILE"
fi

confirm() {
    if [[ "${CONFIRM:-}" == "yes" ]]; then
        return 0
    fi
    if [[ ! -t 0 ]]; then
        die "refusing to restore without confirmation; re-run with CONFIRM=yes"
    fi
    printf 'WARNING: this will overwrite objects in database "%s" on %s.\n' "$PGDATABASE" "$MODE"
    printf 'Type "yes" to continue: '
    local reply=""
    read -r reply
    [[ "$reply" == "yes" || "$reply" == "y" || "$reply" == "Y" ]]
}
confirm || die "aborted"

# Emit the (decrypted) dump bytes on stdout.
open_stream() {
    if [[ "$ENCRYPTED" == "1" ]]; then
        age -d -i "$AGE_IDENTITY_FILE" "$BACKUP_FILE"
    else
        cat -- "$BACKUP_FILE"
    fi
}

restore_sql() {
    # stdin = plain SQL
    case "$MODE" in
        k8s)
            kubectl exec -i -n "$NAMESPACE" "$STSET" -- \
                psql -v ON_ERROR_STOP=1 -U "$PGUSER" -d "$PGDATABASE"
            ;;
        direct)
            psql -v ON_ERROR_STOP=1 \
                -h "$PGHOST" -p "${PGPORT:-5432}" -U "$PGUSER" -d "$PGDATABASE"
            ;;
    esac
}

restore_archive() {
    # stdin = custom-format archive
    case "$MODE" in
        k8s)
            kubectl exec -i -n "$NAMESPACE" "$STSET" -- \
                pg_restore --clean --if-exists --no-owner -U "$PGUSER" -d "$PGDATABASE"
            ;;
        direct)
            pg_restore --clean --if-exists --no-owner \
                -h "$PGHOST" -p "${PGPORT:-5432}" -U "$PGUSER" -d "$PGDATABASE"
            ;;
    esac
}

log "Restoring $BACKUP_FILE into $PGDATABASE (mode=$MODE, format=$EFFECTIVE)"

case "$EFFECTIVE" in
    *.sql.gz)
        open_stream | gunzip -c | restore_sql
        ;;
    *.dump|*.backup)
        open_stream | restore_archive
        ;;
    *.sql)
        open_stream | restore_sql
        ;;
    *)
        die "unsupported backup format: $BACKUP_FILE"
        ;;
esac

log "Restore complete."
