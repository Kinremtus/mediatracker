#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

REGISTRY="ghcr.io/kinremtus/mediatracker"
TAG="${1:-latest}"
IMAGE="$REGISTRY:$TAG"
SSH_HOST="${2:-VPS1}"

echo "==> Building $IMAGE ..."
DOCKER_BUILDKIT=1 docker build --network=host -t "$IMAGE" .

echo ""
echo "==> Pushing to GHCR ..."
docker push "$IMAGE"

echo ""
echo "==> Deploying on $SSH_HOST ..."
ssh "$SSH_HOST" -- "
  set -e
  cd ~/mediatracker
  sudo helm --kubeconfig /etc/rancher/k3s/k3s.yaml upgrade --install app chart/ \
    --namespace mediatracker \
    --set image.tag=$TAG
  sudo kubectl rollout status deployment/app -n mediatracker --timeout=120s
"

echo ""
echo "==> Done! App updated to $IMAGE"
