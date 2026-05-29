#!/bin/bash
set -euo pipefail

if [ -z "${1:-}" ]; then
    echo "Usage: $0 <backup_file.sql.gz>"
    echo ""
    echo "Available backups:"
    ls -lh ./backups/*.sql.gz 2>/dev/null || echo "  No backups found"
    exit 1
fi

BACKUP_FILE="$1"

if [ ! -f "$BACKUP_FILE" ]; then
    echo "Error: File not found: $BACKUP_FILE"
    exit 1
fi

echo "WARNING: This will overwrite the current database!"
echo "Backup file: $BACKUP_FILE"
read -p "Continue? (y/N) " -n 1 -r
echo

if [[ ! $REPLY =~ ^[Yy]$ ]]; then
    echo "Aborted."
    exit 0
fi

echo "[$(date)] Restoring from $BACKUP_FILE..."

gunzip -c "$BACKUP_FILE" | docker compose exec -T db psql -U Kin -d tracker

echo "[$(date)] Restore complete."
