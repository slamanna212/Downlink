#!/usr/bin/env bash
# One command: synthetic stream server + test Dispatcharr + idempotent bootstrap.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
"$HERE/../fixtures/streams/streams-up.sh"
"$HERE/dispatcharr-up.sh"
python3 -I "$HERE/bootstrap-dispatcharr.py"
echo
echo "Dispatcharr:  $(grep ^DISPATCHARR_URL "$HERE/.env.local" | cut -d= -f2)   (credentials + API key in $HERE/.env.local)"
echo "Streams/M3U/EPG: http://localhost:${STREAM_PORT:-8088}/playlist.m3u  /epg.xml"
