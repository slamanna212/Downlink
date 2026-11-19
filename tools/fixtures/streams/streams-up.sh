#!/usr/bin/env bash
# Start (or report) the synthetic stream server in the background. PID/log under tools/dev/.state/.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
STATE="$HERE/../../dev/.state"; mkdir -p "$STATE"
PORT="${STREAM_PORT:-8088}"
[ -f "$HERE/../media/h264_aac_1080p30.ts" ] || "$HERE/make-media.sh"
if [ -f "$STATE/streams.pid" ] && kill -0 "$(cat "$STATE/streams.pid")" 2>/dev/null; then
  echo "stream server already running (pid $(cat "$STATE/streams.pid")) on :$PORT"; exit 0
fi
nohup setsid python3 "$HERE/serve.py" --port "$PORT" >"$STATE/streams.log" 2>&1 &
echo $! >"$STATE/streams.pid"
for _ in $(seq 1 20); do curl -fsS "http://localhost:$PORT/health" >/dev/null 2>&1 && { echo "stream server up on :$PORT (pid $(cat "$STATE/streams.pid"))"; exit 0; }; sleep 0.5; done
echo "failed to start; see $STATE/streams.log"; exit 1
