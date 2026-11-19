#!/usr/bin/env bash
# Start the local test Dispatcharr (PLAN 11.6). Idempotent. Synthetic content only.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
IMAGE="ghcr.io/dispatcharr/dispatcharr:0.34.1"   # pinned; bump deliberately and re-capture fixtures
NAME="${DISPATCHARR_CONTAINER:-dispatcharr-test}"
PORT="${DISPATCHARR_PORT:-9191}"
VOLUME="${DISPATCHARR_VOLUME:-dispatcharr_test}"

if docker ps --format '{{.Names}}' | grep -qx "$NAME"; then
  echo "$NAME already running"
elif docker ps -a --format '{{.Names}}' | grep -qx "$NAME"; then
  docker start "$NAME" >/dev/null
  echo "$NAME started"
else
  docker run -d --name "$NAME" -p "127.0.0.1:${PORT}:9191" -v "${VOLUME}:/data" \
    --add-host=host.docker.internal:host-gateway "$IMAGE" >/dev/null
  echo "$NAME created from $IMAGE"
fi

echo -n "waiting for http://localhost:${PORT} "
for _ in $(seq 1 120); do
  if curl -fsS -o /dev/null "http://localhost:${PORT}/api/core/version/" 2>/dev/null; then echo " up"; exit 0; fi
  echo -n .; sleep 2
done
echo " TIMEOUT (docker logs $NAME)"; exit 1
