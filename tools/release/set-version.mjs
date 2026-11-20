// Inject the release version (from the root package.json, or an explicit arg) into every file that
// carries one, at BUILD time (never committed): Cargo.toml [workspace.package], apps/desktop/tauri.conf.json,
// apps/ui/package.json. Keeps them from drifting from the root package.json (the single source of truth).
// Usage: node tools/release/set-version.mjs [X.Y.Z[-pre]]
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const version = process.argv[2] ?? JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version;
if (!/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/.test(version)) {
  console.error(`invalid version '${version}'`);
  process.exit(2);
}

function edit(rel, fn) {
  const p = join(root, rel);
  const before = readFileSync(p, "utf8");
  const after = fn(before);
  if (after === before && !before.includes(`"${version}"`)) {
    console.error(`no version field replaced in ${rel}`);
    process.exit(1);
  }
  writeFileSync(p, after);
  console.log(`${rel} -> ${version}`);
}

// Top-level 2-space-indented "version" in the JSON files (formatting preserved).
const json = (s) => s.replace(/^(  "version"\s*:\s*)"[^"]*"/m, `$1"${version}"`);
edit("apps/desktop/tauri.conf.json", json);
edit("apps/ui/package.json", json);

// First `version = "..."` inside [workspace.package].
edit("Cargo.toml", (s) =>
  s.replace(/(\[workspace\.package\][^\[]*?\nversion\s*=\s*)"[^"]*"/, `$1"${version}"`),
);
