#!/usr/bin/env python3
"""Build the Tauri updater manifest (latest.json) from release assets (PLAN §10.3).

tauri-action only emits the NSIS/AppImage/app.tar.gz keys and races when several jobs
upload to one release, so the workflow collects every asset + .sig and calls this instead.

Keys produced:
  windows-x86_64          Downlink_<ver>_x64-setup.exe
  darwin-aarch64          Downlink_<ver>_universal.app.tar.gz
  darwin-x86_64           Downlink_<ver>_universal.app.tar.gz
  linux-x86_64            Downlink_<ver>_amd64.AppImage
  linux-x86_64-deb        Downlink_<ver>_amd64.deb

darwin-* keys only with --macos (macOS builds are not in the pipeline yet).

Usage: make-latest-json.py --version 0.2.0 --repo owner/name --tag v0.2.0 \
         --assets DIR [--notes FILE] [--macos] [--out latest.json]
Fails if any platform asset or its .sig is missing.
"""
import argparse
import datetime
import json
import pathlib
import sys

PATTERNS = {
    "windows-x86_64": ["*_x64-setup.exe"],
    "darwin-aarch64": ["*_universal.app.tar.gz", "*.app.tar.gz"],
    "darwin-x86_64": ["*_universal.app.tar.gz", "*.app.tar.gz"],
    "linux-x86_64": ["*_amd64.AppImage"],
    "linux-x86_64-deb": ["*_amd64.deb"],
}


def find(assets: pathlib.Path, globs):
    for g in globs:
        hits = sorted(p for p in assets.glob(g) if p.is_file())
        if hits:
            return hits[0]
    return None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--version", required=True, help="version without leading v (may be X.Y.Z-dev.sha)")
    ap.add_argument("--repo", required=True)
    ap.add_argument("--tag", required=True, help="release tag the asset URLs point at")
    ap.add_argument("--assets", required=True, type=pathlib.Path)
    ap.add_argument("--notes", type=pathlib.Path)
    ap.add_argument("--macos", action="store_true", help="include darwin-* keys (needs the universal .app.tar.gz)")
    ap.add_argument("--out", type=pathlib.Path, default=pathlib.Path("latest.json"))
    a = ap.parse_args()

    platforms, missing = {}, []
    for key, globs in PATTERNS.items():
        if not a.macos and key.startswith("darwin-"):
            continue
        f = find(a.assets, globs)
        sig = pathlib.Path(str(f) + ".sig") if f else None
        if not f or not sig.is_file():
            missing.append(key)
            continue
        platforms[key] = {
            "signature": sig.read_text(encoding="utf-8").strip(),
            "url": f"https://github.com/{a.repo}/releases/download/{a.tag}/{f.name}",
        }
    if missing:
        print("missing asset or .sig for: " + ", ".join(missing), file=sys.stderr)
        return 1

    manifest = {
        "version": a.version,
        "notes": a.notes.read_text(encoding="utf-8") if a.notes and a.notes.is_file() else "",
        "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "platforms": platforms,
    }
    a.out.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    print(f"wrote {a.out} with {len(platforms)} platforms")
    return 0


if __name__ == "__main__":
    sys.exit(main())
