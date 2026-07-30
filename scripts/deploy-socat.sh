#!/usr/bin/env bash
# deploy-socat.sh
#
# Creates systemd socat units on the VPS to forward ports from the WireGuard
# interface (10.6.0.1) to localhost NodePorts.
#
# Workaround for nftables FORWARD policy drop -- socat listens on wg0 IP,
# forwards to localhost, traffic goes through INPUT instead of FORWARD.
#
# Usage:
#   sudo ./deploy-socat.sh              # create units and start all
#   sudo ./deploy-socat.sh --dry-run    # show what would be created
#   sudo ./deploy-socat.sh --stop       # stop and disable all
#
# Safe to re-run -- overwrites existing unit files and reloads.

set -euo pipefail

DRY_RUN=0
if [[ "${1:-}" == "--dry-run" ]]; then
    DRY_RUN=1
fi

# Format: listen_port:node_port:service_name
SERVICES=(
    "9090:30909:prometheus"
    "3000:30001:grafana"
    "3100:32310:loki"
    "9093:30903:alertmanager"
    "19000:31904:kube-state-metrics"
)

WG_IP="10.6.0.1"

create_unit() {
    local listen_port="$1"
    local node_port="$2"
    local service_name="$3"
    local unit_file="/etc/systemd/system/socat-${service_name}.service"

    if [[ "$DRY_RUN" -eq 1 ]]; then
        echo "[DRY-RUN] Would create: $unit_file"
        echo "  ExecStart=/usr/bin/socat TCP-LISTEN:${listen_port},bind=${WG_IP},fork,reuseaddr TCP:127.0.0.1:${node_port}"
        return
    fi

    cat > "$unit_file" <<UNIT
[Unit]
Description=socat forward ${service_name} (${listen_port} -> ${node_port})
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/bin/socat TCP-LISTEN:${listen_port},bind=${WG_IP},fork,reuseaddr TCP:127.0.0.1:${node_port}
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
UNIT

    chmod 644 "$unit_file"
    echo "  [OK] Created $unit_file"
}

# --- main ---

if [[ "$EUID" -ne 0 ]]; then
    echo "[!] This script must be run as root (creates systemd units)."
    exit 1
fi

echo "==> socat units: create + enable"

for service_spec in "${SERVICES[@]}"; do
    IFS=':' read -r listen_port node_port service_name <<< "$service_spec"
    echo ""
    echo "  Service: $service_name"
    echo "    listen: ${WG_IP}:${listen_port}"
    echo "    forward: 127.0.0.1:${node_port}"
    create_unit "$listen_port" "$node_port" "$service_name"
done

if [[ "$DRY_RUN" -eq 1 ]]; then
    echo ""
    echo "[DRY-RUN] Run without --dry-run to create and enable units."
    exit 0
fi

echo ""
echo "==> Reloading systemd..."
systemctl daemon-reload

echo ""
echo "==> Enabling and starting all units..."
for service_spec in "${SERVICES[@]}"; do
    IFS=':' read -r _ _ service_name <<< "$service_spec"
    echo "  socat-${service_name}.service"
    systemctl enable --now "socat-${service_name}.service"
done

echo ""
echo "==> Status check:"
systemctl is-active socat-{prometheus,grafana,kube-state-metrics,loki,alertmanager} 2>/dev/null || true

echo ""
echo "=== Done ==="
echo "Verify from phone:"
echo "  curl http://${WG_IP}:9090/metrics | head -5"
echo "  curl http://${WG_IP}:3000"
echo "  curl http://${WG_IP}:19000/metrics | head -5"
echo ""
echo "Stop all: sudo systemctl stop socat-{prometheus,grafana,kube-state-metrics,loki,alertmanager}"
echo "Disable:  sudo systemctl disable socat-{prometheus,grafana,kube-state-metrics,loki,alertmanager}"
