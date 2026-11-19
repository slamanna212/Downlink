#!/usr/bin/env bash
# Stop the test Dispatcharr. Pass --purge to also delete the container and its data volume.
set -euo pipefail
NAME="${DISPATCHARR_CONTAINER:-dispatcharr-test}"
VOLUME="${DISPATCHARR_VOLUME:-dispatcharr_test}"
docker stop "$NAME" 2>/dev/null || true
if [ "${1:-}" = "--purge" ]; then
  docker rm "$NAME" 2>/dev/null || true
  docker volume rm "$VOLUME" 2>/dev/null || true
  rm -f "$(dirname "$0")/.env.local"
  echo "purged container, volume and .env.local"
fi
