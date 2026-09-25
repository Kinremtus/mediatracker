#!/usr/bin/env python3
"""Thin wrapper around scripts/backup-db.sh.

Historically this file produced the backup itself with a ``shell=True``
invocation of ``docker compose exec`` and had a broken final print
(``TITIMESTAMPME``). Both problems are gone: all backup logic now lives in
``scripts/backup-db.sh`` (k3s/direct detection, age encryption, retention,
rclone is handled by the in-cluster CronJob) and this wrapper only delegates.

No ``shell=True`` is used anywhere; arguments are passed as a list.
The Minsk (Europe/Minsk) timestamp style is kept for the log lines.
"""

from __future__ import annotations

import subprocess
import sys
from datetime import datetime
from pathlib import Path
from zoneinfo import ZoneInfo

SCRIPT_DIR = Path(__file__).resolve().parent
BACKUP_SCRIPT = SCRIPT_DIR / "backup-db.sh"


def now() -> str:
    return datetime.now(ZoneInfo("Europe/Minsk")).strftime("%d.%m.%Y_%H-%M-%S")


def main() -> int:
    timestamp = now()
    print(f"[{timestamp}] Starting backup via {BACKUP_SCRIPT} ...")

    if not BACKUP_SCRIPT.is_file():
        print(f"[{timestamp}] ERROR: backup script not found: {BACKUP_SCRIPT}", file=sys.stderr)
        return 1

    try:
        result = subprocess.run(
            ["bash", str(BACKUP_SCRIPT), *sys.argv[1:]],
            check=False,
        )
    except FileNotFoundError as exc:
        print(f"[{timestamp}] ERROR: {exc}", file=sys.stderr)
        return 1

    if result.returncode != 0:
        print(
            f"[{timestamp}] ERROR: backup failed (exit code {result.returncode})",
            file=sys.stderr,
        )
        return result.returncode

    print(f"[{timestamp}] Done.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
