#!/usr/bin/env bash
# Bump the Downlink release version. The ROOT package.json is the source of truth; tauri.conf.json, Cargo.toml,
# Cargo.lock and apps/ui/package.json are synced from it here too, so local builds (User-Agent, About) report the
# right version. CI re-injects the version at build time anyway (tools/release/set-version.mjs).
# Usage: tools/release/bump-version.sh 0.2.0      (X.Y.Z or X.Y.Z-pre, no leading "v")
# Then: git commit -am "chore: release v0.2.0" && git push   (a push to main publishes it)
set -euo pipefail

VER="${1:?usage: $0 X.Y.Z[-pre]}"
if ! [[ "$VER" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "version must be X.Y.Z or X.Y.Z-pre (got '$VER')" >&2
  exit 2
fi
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
node -e '
const fs = require("fs");
const p = JSON.parse(fs.readFileSync("package.json", "utf8"));
p.version = process.argv[1];
fs.writeFileSync("package.json", JSON.stringify(p, null, 2) + "\n");
' "$VER"
node tools/release/set-version.mjs "$VER"
cargo update -w --offline --quiet
echo "package.json version is now $(node -p "require('./package.json').version")"
