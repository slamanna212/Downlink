#!/usr/bin/env bash
# Run the M0 spike: a self-contained page served by the embedded asset protocol.
# Usage: apps/desktop/spike/run.sh [extra env, e.g. DOWNLINK_SPIKE_URLS=... DOWNLINK_SPIKE_QUIT=1]
# Dev only: DOWNLINK_MPV_PATH defaults to the distro libmpv if no bundled engine is staged.
set -euo pipefail
cd "$(dirname "$0")/../../.."
export DOWNLINK_MPV_PATH="${DOWNLINK_MPV_PATH:-/usr/lib/x86_64-linux-gnu/libmpv.so.2}"
export TAURI_CONFIG='{"build":{"frontendDist":"./spike"}}'
exec cargo run -p downlink-desktop --features spike "$@"
