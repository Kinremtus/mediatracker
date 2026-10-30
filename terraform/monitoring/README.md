# Monitoring Stack (K3s + Terraform)

Production monitoring stack deployed on a single-node K3s cluster via Terraform.

## Components

| Component | Type | Image | Function |
|-----------|------|-------|----------|
| **Prometheus** | Deployment | `prom/prometheus` | Metrics storage, alert evaluation, 7d retention |
| **Loki** | StatefulSet | `grafana/loki:3.7.3` | Log aggregation (TSDB schema v13) |
| **Alertmanager** | Deployment | `prom/alertmanager` | Alert routing, Telegram notifications |
| **kube-state-metrics** | Deployment | `registry.k8s.io/kube-state-metrics` | K8s object metrics |
| **node-exporter** | DaemonSet | `prom/node-exporter` | Node-level metrics (CPU, RAM, disk, net) |
| **Promtail** | DaemonSet | `grafana/promtail` | Log shipping, CRI pipeline for containerd |

> **2026-10-30:** Grafana и Uptime Kuma вынесены с VPS (узел 1 vCPU / 2 GiB
> свопил). Grafana теперь дома в Docker (`:3002`), Uptime Kuma заменена на
> UptimeRobot free. См. раздел «Перенос Grafana и Uptime Kuma».

## Architecture

```
Prometheus :9090  <-- scrape: app, node-exporter, kube-state-metrics
  |
  ├-- Alertmanager :9093    (alerts -> Telegram)
  └-- Loki :3100            (logs from Promtail DaemonSet)

Grafana (дома, Docker :3002) --SSH-туннель--> 127.0.0.1:19090 (Prometheus)
                                               127.0.0.1:13100 (Loki)
```

Data is persistent via `local-path` PVCs:
- Prometheus: 5Gi
- Loki: 5Gi (StatefulSet with volumeClaimTemplates)
- Grafana: 1Gi и Uptime Kuma: 1Gi — **сохранены, но отвязаны** (данные старых
  инсталляций; можно удалить, когда дашборды/мониторы больше не нужны)

## Access (WireGuard ingress)

The cluster is behind an nftables firewall with `policy drop` on FORWARD.
kube-proxy runs in `iptables-legacy` mode, creating DNAT rules that don't
interact with the nftables FORWARD chain -- traffic from WireGuard (wg0)
gets dropped before reaching kube-proxy rules.

**Workaround:** socat on the VPS listens on the WireGuard interface IP
and forwards to localhost, where kube-proxy handles DNAT via INPUT (not FORWARD).

### socat port mapping

| Service | NodePort | socat listen (10.6.0.1) | URL (from phone) |
|---------|----------|------------------------|-------------------|
| Prometheus | 30909 | :9090 | `http://10.6.0.1:9090/metrics` |
| Loki | 32310 | :3100 | `http://10.6.0.1:3100/ready` |
| Alertmanager | 30903 | :9093 | `http://10.6.0.1:9093/-/ready` |
| kube-state-metrics | 31904 | :19000 | `http://10.6.0.1:19000/metrics` |

Note: kube-state-metrics uses port **19000** (not 31904) because the NodePort
port itself is intercepted by kube-proxy DNAT in PREROUTING, which sends
traffic through FORWARD (dropped). A different port avoids this.

### socat systemd units

Each service gets its own systemd unit:

```
/etc/systemd/system/socat-prometheus.service
/etc/systemd/system/socat-loki.service
/etc/systemd/system/socat-alertmanager.service
/etc/systemd/system/socat-kube-state-metrics.service
```

Usage:
```bash
# Deploy all units (run on VPS):
sudo systemctl daemon-reload
sudo systemctl enable --now socat-{prometheus,kube-state-metrics,loki,alertmanager}

# Check status:
sudo systemctl status socat-{prometheus,kube-state-metrics,loki,alertmanager}
```

## Deployment

Terraform is applied from a laptop through an SSH tunnel:

```bash
# On laptop, establish tunnel:
ssh -L 16443:127.0.0.1:6443 user@vps

# Apply monitoring stack:
cd terraform/monitoring
terraform init -backend-config=backend.r2.example.hcl   # state lives in R2
terraform plan
terraform apply
```

### Prerequisites

- `~/.kube/config-vps` pointing to `https://127.0.0.1:16443`
- Terraform >= 1.5
- Telegram bot token in `telegram-secret.tf`

## Alerts

| Alert | Condition | Severity | Action |
|-------|-----------|----------|--------|
| AppDown | `up{app="app"} == 0` for 1m | critical | Telegram |
| HighCPU | `rate(process_cpu_seconds_total{app="app"}[5m]) > 0.8` for 2m | warning | Telegram |
| BackupJobFailed | `kube_job_status_failed{job_name=~"postgres-backup-.*"} > 0` for 5m | critical | Telegram |
| BackupStale | `time() - kube_cronjob_status_last_successful_time{cronjob="postgres-backup"} > 26h` for 30m | critical | Telegram |

## Files

```
terraform/monitoring/
  providers.tf           -- Kubernetes provider (insecure, tunnel)
  monitoring.tf          -- namespace
  monitoring-rbac.tf     -- ClusterRole + binding
  prometheus.tf          -- Prometheus (deployment, service, PVC, config)
  loki.tf                -- Loki (StatefulSet, service, config)
  alertmanager.tf        -- Alertmanager (deployment, service, Telegram config)
  kube-state-metrics.tf  -- kube-state-metrics (deployment, service, RBAC)
  node-exporter.tf       -- Node exporter (DaemonSet, hostNetwork)
  promtail.tf            -- Promtail (DaemonSet, log shipping)
  telegram-secret.tf     -- Telegram bot token (gitignored)
  backend.r2.example.hcl -- partial S3/R2 backend config (state offsite)
```

## Перенос Grafana и Uptime Kuma (2026-10-30)

Узел 1 vCPU / 2 GiB свопил (доступно ~300 МБ), из-за чего страницы грузились
медленно. Тяжёлые части мониторинга вынесены:

- **Grafana** — домашняя машина, Docker `grafana/grafana:13.2.5`, `--network
  host`, порт `3002`, `--user 1000:1000`. Datasources:
  `http://127.0.0.1:19090` (Prometheus) и `http://127.0.0.1:13100` (Loki).
  Доступ к VPS — SSH-туннель: user-юнит `monitoring-tunnel.service`
  (`-L 19090:127.0.0.1:30909 -L 13100:127.0.0.1:32310 VPS1`), автозапуск
  включён; для работы без логина — `sudo loginctl enable-linger $USER`.
  Пароль admin — в `~/monitoring/grafana/admin-password.txt` (chmod 600).
- **Uptime Kuma** — заменена на **UptimeRobot free** (внешние проверки 24/7):
  монитор HTTPS на `https://<APP_HOST>/health`, keyword `ok`, интервал 5 мин.
- **Осталось на VPS**: Prometheus (44 МБ), Loki (61), Alertmanager, Promtail,
  kube-state-metrics, node-exporter. Алерты в Telegram не зависят от домашней
  машины.

Освобождено ~340 МБ RAM; `free -m` после: used 1298 (было 1670), available 668
(было 326).

**Terraform state:** `grafana.tf` и `uptime-kuma.tf` удалены из конфигурации,
ресурсы в кластере удалены вручную. Перед следующим `terraform apply` уберите
их из state, иначе Terraform попробует удалять несуществующее:

```bash
terraform state rm kubernetes_persistent_volume_claim_v1.grafana \
  kubernetes_service_v1.grafana kubernetes_config_map_v1.grafana \
  kubernetes_deployment_v1.grafana \
  kubernetes_persistent_volume_claim_v1.uptime-kuma \
  kubernetes_service_v1.uptime-kuma kubernetes_deployment_v1.uptime-kuma
```

PVC `grafana-data` (339 МБ) и `uptime-kuma-data` (48 МБ) намеренно сохранены —
там старая БД Grafana (дашборды) и мониторы Uptime Kuma; удалить можно позже.

## Terraform state on R2

The state file is **not** in git (`.gitignore` covers `terraform/**/*.tfstate`).
`providers.tf` declares the empty S3 backend and every real parameter lives in
`backend.r2.example.hcl`, so nothing sensitive is committed. The one-time
migration (state = the single source of truth for this stack, keep a manual
copy until it has run):

```bash
# 1. create the R2 bucket + an API token (Object Read & Write)
export AWS_ACCESS_KEY_ID=<token key>
export AWS_SECRET_ACCESS_KEY=<token secret>
# 2. fill in the endpoint in backend.r2.example.hcl, then:
terraform init -backend-config=backend.r2.example.hcl -migrate-state
```

After the migration every `plan/apply` reads and writes state in R2 (same
bucket as the DB dumps, different prefix).

## Troubleshooting

```bash
# Check pod status:
kubectl -n monitoring get pods

# Forward port for debugging:
kubectl -n monitoring port-forward svc/prometheus 9090:9090

# Check nftables rules (may interfere):
sudo nft list ruleset

# Check iptables mode:
sudo iptables --version

# Test socat health (from VPS):
curl -s http://localhost:19000/metrics | head -5
curl -s http://localhost:9090/metrics | head -5
```
