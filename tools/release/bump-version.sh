#!/usr/bin/env bash
# Bump the Downlink version everywhere in one go (PLAN §10.4):
#   Cargo.toml [workspace.package] (all crates inherit it), apps/desktop/tauri.conf.json,
#   apps/ui/package.json, and Cargo.lock.
# Usage: tools/release/bump-version.sh 0.2.0     (plain semver X.Y.Z, no leading "v")
set -euo pipefail

VER="${1:?usage: $0 X.Y.Z}"
if ! [[ "$VER" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "version must be X.Y.Z (got '$VER')" >&2
  exit 2
fi
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

# Cargo.toml: only the version line inside [workspace.package].
python3 - "$VER" <<'PY'
import re, sys
ver = sys.argv[1]
p = "Cargo.toml"
s = open(p, encoding="utf-8").read()
m = re.search(r"(?ms)^\[workspace\.package\]\n(.*?)(?=^\[|\Z)", s)
if not m:
    sys.exit("no [workspace.package] in Cargo.toml")
block, n = re.subn(r'(?m)^version\s*=\s*"[^"]*"', f'version = "{ver}"', m.group(1), count=1)
if n != 1:
    sys.exit("no version in [workspace.package]")
open(p, "w", encoding="utf-8").write(s[: m.start(1)] + block + s[m.end(1):])

# JSON files: targeted regex on the top-level "version" so formatting is preserved.
for p in ("apps/desktop/tauri.conf.json", "apps/ui/package.json"):
    s = open(p, encoding="utf-8").read()
    s, n = re.subn(r'(?m)^(\s{2}"version"\s*:\s*)"[^"]*"', rf'\g<1>"{ver}"', s, count=1)
    if n != 1:
        sys.exit(f"no top-level version in {p}")
    open(p, "w", encoding="utf-8").write(s)
PY

# Refresh the workspace members' entries in Cargo.lock without touching third-party pins.
cargo update --workspace --offline >/dev/null 2>&1 || cargo metadata --format-version 1 >/dev/null

echo "bumped to $VER:"
grep -n '^version' Cargo.toml | head -1
grep -n '"version"' apps/desktop/tauri.conf.json apps/ui/package.json | head -2
echo "next: git commit -am 'chore: release v$VER' && git tag v$VER"
