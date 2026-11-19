#!/usr/bin/env bash
# Stop stream server and Dispatcharr (data kept). Use dispatcharr-down.sh --purge to wipe.
HERE="$(cd "$(dirname "$0")" && pwd)"
"$HERE/../fixtures/streams/streams-down.sh"; "$HERE/dispatcharr-down.sh"
