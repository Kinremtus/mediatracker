# Monitoring Stack (K3s + Terraform)

Production monitoring stack deployed on a single-node K3s cluster via Terraform.

## Components

| Component | Type | Image | Function |
|-----------|------|-------|----------|
| **Prometheus** | Deployment | `prom/prometheus` | Metrics storage, alert evaluation, 7d retention |
| **Grafana** | Deployment | `grafana/grafana` | Dashboards, pre-configured datasources |
| **Loki** | StatefulSet | `grafana/loki:3.7.3` | Log aggregation (TSDB schema v13) |
| **Alertmanager** | Deployment | `prom/alertmanager` | Alert routing, Telegram notifications |
| **kube-state-metrics** | Deployment | `registry.k8s.io/kube-state-metrics` | K8s object metrics |
| **node-exporter** | DaemonSet | `prom/node-exporter` | Node-level metrics (CPU, RAM, disk, net) |
| **Promtail** | DaemonSet | `grafana/promtail` | Log shipping, CRI pipeline for containerd |
| **Uptime Kuma** | Deployment | `louislam/uptime-kuma:1` | Uptime monitoring, status pages, alerting |

## Architecture

```
Prometheus :9090  <-- scrape: app, node-exporter, kube-state-metrics
  |
  ├-- Grafana :3000         (datasources: Prometheus, Loki)
  ├-- Alertmanager :9093    (alerts -> Telegram)
  └-- Loki :3100            (logs from Promtail DaemonSet)
```

Data is persistent via `local-path` PVCs:
- Prometheus: 5Gi
- Grafana: 1Gi
- Loki: 5Gi (StatefulSet with volumeClaimTemplates)

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
| Grafana | 30001 | :3000 | `http://10.6.0.1:3000` |
| Loki | 32310 | :3100 | `http://10.6.0.1:3100/ready` |
| Alertmanager | 30903 | :9093 | `http://10.6.0.1:9093/-/ready` |
| kube-state-metrics | 31904 | :19000 | `http://10.6.0.1:19000/metrics` |
| Uptime Kuma | 30011 | :3001 | `http://10.6.0.1:3001` |

Note: kube-state-metrics uses port **19000** (not 31904) because the NodePort
port itself is intercepted by kube-proxy DNAT in PREROUTING, which sends
traffic through FORWARD (dropped). A different port avoids this.

### socat systemd units

Each service gets its own systemd unit:

```
/etc/systemd/system/socat-prometheus.service
/etc/systemd/system/socat-grafana.service
/etc/systemd/system/socat-loki.service
/etc/systemd/system/socat-alertmanager.service
/etc/systemd/system/socat-kube-state-metrics.service
/etc/systemd/system/socat-uptime-kuma.service
```

Usage:
```bash
# Deploy all units (run on VPS):
sudo systemctl daemon-reload
sudo systemctl enable --now socat-{prometheus,grafana,kube-state-metrics,loki,alertmanager,uptime-kuma}

# Check status:
sudo systemctl status socat-{prometheus,grafana,kube-state-metrics,loki,alertmanager,uptime-kuma}
```

## Deployment

Terraform is applied from a laptop through an SSH tunnel:

```bash
# On laptop, establish tunnel:
ssh -L 16443:127.0.0.1:6443 user@vps

# Apply monitoring stack:
cd terraform/monitoring
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
  grafana.tf             -- Grafana (deployment, service, PVC, datasources)
  loki.tf                -- Loki (StatefulSet, service, config)
  alertmanager.tf        -- Alertmanager (deployment, service, Telegram config)
  kube-state-metrics.tf  -- kube-state-metrics (deployment, service, RBAC)
  node-exporter.tf       -- Node exporter (DaemonSet, hostNetwork)
  promtail.tf            -- Promtail (DaemonSet, log shipping)
  uptime-kuma.tf         -- Uptime Kuma (deployment, service, PVC)
  telegram-secret.tf     -- Telegram bot token (gitignored)
  backend.r2.example.hcl -- partial S3/R2 backend config (state offsite)
```

## Terraform state on R2 (recommended)

The state file is **not** in git (`.gitignore` covers `terraform/**/*.tfstate`)
and currently lives only on the workstation that ran `apply`. Losing it means
drift plus manual `terraform import` of every object, and the file contains
secrets. `backend.r2.example.hcl` moves it into Cloudflare R2 (same bucket as
the DB dumps, different prefix):

```bash
# 1. add `backend "s3" {}` to the terraform block in providers.tf
# 2. create the R2 bucket + an API token (Object Read & Write)
export AWS_ACCESS_KEY_ID=<token key>
export AWS_SECRET_ACCESS_KEY=<token secret>
# 3. fill in the endpoint in backend.r2.example.hcl, then:
terraform init -backend-config=backend.r2.example.hcl -migrate-state
```

Until the backend is enabled, keep a manual copy of `terraform.tfstate`
somewhere safe (it is the only source of truth for this stack).

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
