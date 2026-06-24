#!/usr/bin/env bash
set -euo pipefail

# Перейти в корень репозитория (родитель папки scripts/)
cd "$(dirname "$0")/.."

REGISTRY="ghcr.io/kinremtus/mediatracker"
TAG="${1:-latest}"
IMAGE="$REGISTRY:$TAG"
SSH_HOST="${2:-VPS1}"

echo "==> Building $IMAGE ..."
DOCKER_BUILDKIT=1 docker build -t "$IMAGE" .

echo ""
echo "==> Pushing to GHCR ..."
docker push "$IMAGE"

echo ""
echo "==> Deploying on $SSH_HOST ..."
ssh "$SSH_HOST" -- "
  set -e
  sudo kubectl set image deployment/app -n mediatracker app=$IMAGE
  sudo kubectl rollout status deployment/app -n mediatracker --timeout=120s
"

echo ""
echo "==> Done! App updated to $IMAGE"
