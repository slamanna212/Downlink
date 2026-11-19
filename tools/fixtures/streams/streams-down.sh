#!/usr/bin/env bash
STATE="$(cd "$(dirname "$0")" && pwd)/../../dev/.state"
[ -f "$STATE/streams.pid" ] && kill "$(cat "$STATE/streams.pid")" 2>/dev/null; rm -f "$STATE/streams.pid"; echo stopped
