#!/usr/bin/env bash
# Allow Prometheus (in-cluster pod network) to scrape node-exporter on :9100.
#
# Why this is needed: node-exporter runs with hostNetwork: true, so its scrape
# address is the NODE's own IP (162.x.x.x:9100), not a pod IP. Prometheus runs
# as a pod and connects from the pod CIDR; the host nftables `input` chain is
# `policy drop`, so those new connections were rejected and the target stayed
# `down` with "context deadline exceeded".
#
# The rule is added to the live ruleset AND persisted in /etc/nftables.conf
# (nftables.service loads that file at boot). Re-running is a no-op.
#
# Usage (on the VPS):
#   sudo ./scripts/vps-firewall-node-exporter.sh
#   POD_CIDR=10.42.0.0/24 PORT=9100 sudo -E ./scripts/vps-firewall-node-exporter.sh
#
# Verify: up{app="node-exporter"} == 1 in Prometheus.
set -euo pipefail

POD_CIDR="${POD_CIDR:-10.42.0.0/24}"
PORT="${PORT:-9100}"
CONF="${CONF:-/etc/nftables.conf}"

[[ "${EUID}" -eq 0 ]] || { echo "[!] run as root (sudo)" >&2; exit 1; }
command -v nft >/dev/null 2>&1 || { echo "[!] nft not found" >&2; exit 1; }
[[ -f "$CONF" ]] || { echo "[!] $CONF not found" >&2; exit 1; }

# 1) Live ruleset: takes effect immediately, no reload (a full `nft -f` would
#    duplicate the dynamic k3s/docker rules that are also baked into the file).
if nft list chain inet filter input | grep -q "dport ${PORT}"; then
    echo "[=] live rule for :${PORT} already present"
else
    nft add rule inet filter input ip saddr "$POD_CIDR" tcp dport "$PORT" accept
    echo "[+] live rule added: ${POD_CIDR} -> :${PORT}"
fi

# 2) Persistence across reboots.
if grep -q "dport ${PORT}" "$CONF"; then
    echo "[=] ${CONF} already contains the rule"
else
    cp -a "$CONF" "${CONF}.bak-$(date +%Y%m%d%H%M%S)"
    POD_CIDR="$POD_CIDR" PORT="$PORT" python3 - "$CONF" <<'PY'
import os, pathlib, sys

path = pathlib.Path(sys.argv[1])
cidr = os.environ["POD_CIDR"]
port = os.environ["PORT"]
rule = f"\t\tip saddr {cidr} tcp dport {port} accept\n"

lines = path.read_text().splitlines(keepends=True)
out, inserted = [], False
for line in lines:
    out.append(line)
    # First occurrence lives in `chain input` (the forward chain has its own).
    if not inserted and "ct state established,related accept" in line:
        out.append(rule)
        inserted = True

if not inserted:
    sys.exit("anchor line 'ct state established,related accept' not found")

path.write_text("".join(out))
print(f"[+] inserted into {path}")
PY
    nft -c -f "$CONF"
    echo "[+] ${CONF} validated (nft -c)"
fi

echo
echo "Current input chain:"
nft list chain inet filter input | grep -E "policy|dport ${PORT}" || true
