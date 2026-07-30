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
```

Usage:
```bash
# Deploy all units (run on VPS):
sudo systemctl daemon-reload
sudo systemctl enable --now socat-{prometheus,grafana,kube-state-metrics,loki,alertmanager}

# Check status:
sudo systemctl status socat-{prometheus,grafana,kube-state-metrics,loki,alertmanager}
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
  telegram-secret.tf     -- Telegram bot token (gitignored)
```

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
