#!/usr/bin/env bash
set -euo pipefail

ENV_FILE="${1:-$HOME/mediatracker/.env}"

kubectl create secret generic app-secret -n mediatracker \
  --from-env-file="$ENV_FILE" \
  --dry-run=client -o yaml | kubectl apply -f -
echo "✓ Secret updated from $ENV_FILE"