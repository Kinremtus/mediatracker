# Postgres backups (k3s)

Daily logical backups of the `tracker` database, run by a Kubernetes CronJob.
The job connects to the `postgres` Service over TCP (host `postgres`, port
`5432`) and writes custom-format dumps (`pg_dump -Fc`) to a PVC. Dumps are
optionally age-encrypted and optionally uploaded offsite with rclone.

```
CronJob postgres-backup (0 3 * * *)
  -> pod: alpine + apk add postgresql17-client age rclone
       -> /scripts/backup.sh   (ConfigMap backup-scripts)
       -> pg_dump -Fc  -->  /backups/YYYYMMDD_HHMMSS.dump[.age]   (PVC backup-data)
       -> age encrypt (if AGE_PUBLIC_KEY)
       -> rclone copy r2:<bucket>   (optional, degrades gracefully)
```

## Files

| File | Resource |
|------|----------|
| `pvc.yaml` | PVC `backup-data` (5Gi, `local-path`, RWO) |
| `configmap.yaml` | ConfigMap `backup-scripts` (the in-cluster `backup.sh`) |
| `cronjob.yaml` | CronJob `postgres-backup` (daily 03:00, `Forbid`) |
| `secret.example.yaml` | Commented example Secret `backup-secret` (no real values) |

The `mediatracker` namespace is assumed to already exist; these manifests do
not create it.

Apply after review (skip `secret.example.yaml` -- it is entirely comments):

```bash
kubectl apply -f k8s/backup/
```

## Required secrets

### db-secret (already exists)

Created outside this directory and consumed via `envFrom`. The backup script
uses libpq-style variables:

| Key | Purpose |
|-----|---------|
| `PGUSER` | database role (fallback: `POSTGRES_USER`) |
| `PGDATABASE` | database name (fallback: `POSTGRES_DB`) |
| `PGHOST` | defaults to `postgres` inside the script |
| `PGPORT` | defaults to `5432` |
| `PGPASSWORD` | read directly by libpq / pg_dump |

### backup-secret (create it yourself)

All keys are optional. A missing key disables a feature instead of failing
the job. See `secret.example.yaml` for the full example and commands.

```
kubectl create secret generic backup-secret -n mediatracker \
  --from-literal=AGE_PUBLIC_KEY='age1...' \
  --from-literal=RCLONE_CONFIG_R2_TYPE='s3' \
  --from-literal=RCLONE_CONFIG_R2_ACCOUNT='<account-id>' \
  --from-literal=RCLONE_CONFIG_R2_KEY='<secret-key>' \
  --from-literal=RCLONE_CONFIG_R2_ENDPOINT='https://<account-id>.r2.cloudflarestorage.com' \
  --from-literal=RCLONE_CONFIG_R2_BUCKET='mediatracker-backups'
```

Generate the age keypair once, on a trusted workstation:

```bash
age-keygen -o age-key.txt      # private key -> keep OFFLINE, never in the cluster
grep 'public key' age-key.txt  # public key -> AGE_PUBLIC_KEY in backup-secret
```

Only the public key is stored in the cluster; decryption needs the private
key (`AGE_IDENTITY_FILE`) during a restore.

## Offsite upload (Cloudflare R2) -- PENDING

The rclone path is wired but **R2 is not configured yet**: no credentials are
in `backup-secret`, so the job logs
`WARNING: rclone/R2 not configured; skipping offsite upload` and keeps the
local dump. Once real R2 credentials exist, uploads start working with no
manifest change.

Uploads degrade gracefully by design: if rclone is missing or the copy fails,
the job logs a warning and still succeeds because the local dump is valid.
Set `BACKUP_REQUIRE_OFFSITE=1` (env in `cronjob.yaml`) once R2 is trusted to
make a failed upload fail the whole job.

## RPO / RTO

| Metric | Value | Notes |
|--------|-------|-------|
| RPO | up to 24h | daily 03:00 schedule; tighten `spec.schedule` to reduce |
| RTO | ~minutes | restore dump + app rollout for the current data size |

Local dumps are pruned after 7 days (`RETENTION_DAYS` in `backup.sh`). Offsite
copies are kept according to the R2 bucket lifecycle policy (set that up in R2).

## Restore procedure

1. Stop writers so the restore is consistent:

   ```bash
   kubectl -n mediatracker scale deployment/app --replicas=0
   ```

2. Get a dump. Either fetch it from the PVC:

   ```bash
   kubectl -n mediatracker run backup-fetch --rm -i --restart=Never \
     --image=alpine:3.21 \
     --overrides='{"spec":{"volumes":[{"name":"b","persistentVolumeClaim":{"claimName":"backup-data"}}],"containers":[{"name":"backup-fetch","image":"alpine:3.21","command":["sh","-c","ls -lh /backups && tar -C /backups -cf - ."],"stdin":true,"volumeMounts":[{"name":"b","mountPath":"/backups"}]}]}}' \
     > backups.tar
   mkdir -p ./backups && tar xf backups.tar -C ./backups
   ```

   or download the newest object from R2 with rclone.

3. Restore with the repo script against a port-forward (keeps credentials out
   of your shell history by reusing `db-secret` values):

   ```bash
   kubectl -n mediatracker port-forward svc/postgres 5432:5432 &
   read -rs -p 'PGPASSWORD: ' PGPASSWORD; export PGPASSWORD
   PGHOST=127.0.0.1 PGUSER=Kin PGDATABASE=tracker \
     AGE_IDENTITY_FILE=./age-key.txt \
     CONFIRM=yes ./scripts/restore-db.sh ./backups/<file>.dump.age
   kill %1
   ```

   The script also supports `*.dump`, `*.sql`, `*.sql.gz` (and their `.age`
   variants) and can restore in-cluster via `USE_K8S=1`.

4. Bring the app back and verify:

   ```bash
   kubectl -n mediatracker scale deployment/app --replicas=1
   kubectl -n mediatracker exec statefulset/postgres -- \
     psql -U Kin -d tracker -c 'SELECT count(*) FROM users;'
   ```

### Drill without touching production

`scripts/restore-drill.sh` (local machine only, needs Docker) starts an
ephemeral Postgres, restores the newest `./backups/*.dump*`, checks that
`users` is non-empty and removes the container. Run it after changing the
backup pipeline.

## Design notes / gotchas

- **Alpine 3.21, not 3.20.** `postgresql17-client` first appears in Alpine
  3.21. Alpine 3.20 only ships pg16 clients, and a pg16 `pg_dump` refuses to
  dump a PostgreSQL 17 server. The CronJob image is therefore `alpine:3.21`.
- **`runAsNonRoot` is disabled here.** The container installs packages with
  `apk add`, which requires root; `runAsNonRoot: true` would prevent the pod
  from starting (the alpine image runs as root). To harden: build a small
  image that already contains `postgresql17-client age rclone`, drop the
  `apk add` step, then set `runAsNonRoot: true`. `allowPrivilegeEscalation:
  false` and `capabilities.drop: [ALL]` are already set.
- If `apk add` ever fails under `capabilities.drop: [ALL]`, add the minimal
  caps it needs (`CHOWN`, `FOWNER`, `DAC_OVERRIDE`, `SETUID`, `SETGID`) back
  to the drop list, or use the prebuilt-image approach above.
- The PVC is `ReadWriteOnce` on `local-path`. With `concurrencyPolicy:
  Forbid` only one backup pod writes at a time; a restore pod must not run at
  the same moment as the CronJob.
- `RCLONE_CONFIG_R2_BUCKET` is used as the destination path (`r2:<bucket>`);
  the other `RCLONE_CONFIG_R2_*` keys define the rclone remote from
  environment variables, so no rclone config file is needed.
- Dumps from the CronJob are owned by `root` on the PVC (the container runs
  as root). This is local storage only; R2 objects get normal permissions.
