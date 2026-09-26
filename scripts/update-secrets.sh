#!/usr/bin/env bash
# Apply the SOPS/age-encrypted secrets to the cluster.
#
# Usage:
#   scripts/update-secrets.sh [secrets-dir]
#
# Each *.enc.yaml is decrypted in a pipe straight into kubectl apply, so the
# plaintext never touches disk. Requires the age private key (see k8s/secrets/README.md).
set -euo pipefail

SECRETS_DIR="${1:-k8s/secrets}"
NAMESPACE="${NAMESPACE:-mediatracker}"
export SOPS_AGE_KEY_FILE="${SOPS_AGE_KEY_FILE:-$HOME/.config/sops/age/keys.txt}"

command -v sops >/dev/null 2>&1 || { echo "[!] missing tool: sops" >&2; exit 1; }
command -v kubectl >/dev/null 2>&1 || { echo "[!] missing tool: kubectl" >&2; exit 1; }
[ -f "$SOPS_AGE_KEY_FILE" ] || { echo "[!] age key not found: $SOPS_AGE_KEY_FILE" >&2; exit 1; }

shopt -s nullglob
files=("$SECRETS_DIR"/*.enc.yaml)
if [ ${#files[@]} -eq 0 ]; then
  echo "[!] no *.enc.yaml found in $SECRETS_DIR" >&2
  exit 1
fi

for f in "${files[@]}"; do
  echo "[..] applying $f"
  sops --decrypt "$f" | kubectl apply -n "$NAMESPACE" -f -
done
echo "[OK] secrets applied to namespace $NAMESPACE"
