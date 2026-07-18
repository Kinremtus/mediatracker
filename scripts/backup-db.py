import subprocess
from datetime import datetime
from zoneinfo import ZoneInfo
from pathlib import Path


BACKUP_DIR="./backups"
TIMESTAMP = datetime.now(ZoneInfo("Europe/Minsk")).strftime("%d.%m.%Y_%H-%M-%S")
BACKUP_FILE = f"{BACKUP_DIR}/{TIMESTAMP}.sql.gz"

Path(BACKUP_DIR).mkdir(parents=True, exist_ok=True)

print(f"[{TIMESTAMP}] Starting backup...")

subprocess.run(f"docker compose exec -T db pg_dump -U Kin tracker | gzip > {BACKUP_FILE}", shell=True, check=True)

size = subprocess.check_output(f"du -h {BACKUP_FILE} | cut -f1", shell=True, text=True).strip()
print(f"[{TIMESTAMP}] Backup saved: {BACKUP_FILE} ({size})")

# Remove backups older than 30 days
subprocess.run('find ./backups -name "*.sql.gz" -mtime +30 -delete', shell=True, check=True)

print(f"[{TIMESTAMP}] Old backups cleaned.")
print(f"[{TITIMESTAMPME}] Done.")