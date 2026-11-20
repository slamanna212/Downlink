# Releasing Downlink

> ## Before the first release: TODO for Sam
>
> Automatic triggers are paused: `ci.yml` and `release.yml` run only via "Run workflow" (workflow_dispatch).
> To re-enable, restore the `pull_request:` trigger in `ci.yml` and the `push:` block in `release.yml`
> (both are noted in comments there).
>
> - [ ] Generate the updater keypair (back it up offline; never commit it):
>   `cargo tauri signer generate -w ~/.tauri/downlink.key`
> - [ ] Add the secrets:
>   `gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/downlink.key`
>   then `gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (it prompts for the value).
> - [ ] Paste the public key (`~/.tauri/downlink.key.pub`) into `plugins.updater.pubkey` in `apps/desktop/tauri.conf.json`.
> - [ ] (Optional) repo variable `LIBMPV_SRC` for Linux. The Windows libmpv pin lives in `tools/mpv/pins.env`; no `MPV_*` variables are needed.
> - [ ] Apply the config in "tauri.conf.json requirements" below and implement the `--self-test` flag.
> - [ ] macOS is OFF by default (releases start as Windows + Linux). See "macOS" below to turn it on.

Implements PLAN §10. Workflows: `.github/workflows/ci.yml` (gates) and `.github/workflows/release.yml`
(build, sign, publish, smoke).

## How a release happens

| Trigger | Channel | Tag | Result |
|---|---|---|---|
| push to `main` | dev | `vX.Y.Z-dev.<shortsha>` (pre-release) | assets on that pre-release; rolling `dev` release gets the newest `latest.json` |
| push tag `vX.Y.Z` | stable | `vX.Y.Z` | full release, unversioned copies, `latest.json` |

Pipeline (`release.yml`):

0. **ci**: calls `ci.yml` as a reusable workflow; nothing below runs unless it passes. `ci.yml` itself
   only triggers on pull requests and via this call, so CI runs once per push.
1. **prepare**: derives version/tag, generates notes with git-cliff, creates a *draft* release. Dev builds
   embed `X.Y.Z-dev.<sha>` as their version (via a merged `--config`) so the updater orders them below
   the eventual `X.Y.Z`. A stable tag must equal the `tauri.conf.json` version or the run fails.
2. **mpv-macos** (arm64 + x86_64 runners) then **build-macos** merges them with `lipo` into a universal tree.
3. **build-windows / build-macos / build-linux**: stage libmpv, build UI, `tauri-action` builds and uploads
   to the draft (`uploadUpdaterJson: false`). Linux additionally signs the deb and rpm (Tauri signs only
   NSIS, AppImage and app.tar.gz), writes `SHA256SUMS-linux-x86_64.txt`, uploads both.
4. **publish**: downloads every asset, builds `latest.json` with `tools/release/make-latest-json.py`
   (all six platform keys of §10.3, fails if any asset or `.sig` is missing), makes unversioned copies
   (stable only), attests provenance, publishes the draft, updates the rolling `dev` release (dev only).
5. **smoke**: installs the published artefacts on a clean runner, asserts no mpv/libmpv, runs
   `--self-test` (see below).

Why our own `latest.json`: parallel `tauri-action` jobs race on one manifest and cannot emit the deb/rpm keys.

### Cutting a stable release

```bash
tools/release/bump-version.sh 0.2.0     # Cargo.toml (workspace), tauri.conf.json, apps/ui/package.json, Cargo.lock
git commit -am "chore: release v0.2.0"
git tag v0.2.0 && git push && git push origin v0.2.0
```

Then hand-edit the release notes (user-visible "What's new" first; git-cliff output below it).
Run `git cliff --current` locally to preview.

## Repository secrets and variables

| Name | Kind | Purpose |
|---|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | secret | minisign private key (contents of the key file) for updater `.sig` files |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | secret | its password |
| `ENABLE_MACOS` | variable | `true` turns on the macOS jobs (libmpv, build, smoke) and the darwin keys in `latest.json`; default off |
| `MACOS_DEVELOPER_ID` | variable | `true` switches macOS builds from ad-hoc to Developer ID + notarisation (needs the Apple secrets) |
| `APPLE_*` | secrets | see "macOS" below; only used when `MACOS_DEVELOPER_ID` is `true` |
| `LIBMPV_SRC` | variable (optional) | absolute path of the `libmpv.so.2` to bundle on Linux; default is the runner's linker cache |

`GITHUB_TOKEN` is used for everything else. Workflow permissions are least-privilege per job
(`contents: write`; `id-token` + `attestations: write` only in `publish`).

## Testing dev builds before the updater key exists

If `TAURI_SIGNING_PRIVATE_KEY` is empty, dev builds still build and publish all installers (NSIS `.exe`,
`.deb`, `.AppImage`, `.rpm`, `.dmg`, plus `SHA256SUMS-linux-x86_64.txt`) to the `vX.Y.Z-dev.<sha>`
pre-release, with a workflow warning annotation. The build gets `bundle.createUpdaterArtifacts=false`
(merged by `.github/actions/setup-build`), no `.sig` files are made, and `latest.json` and the rolling
`dev` release are skipped, so in-app updates do not work until the key is added. A stable tag without the
key fails in `prepare`. Libmpv pins (Windows tag/asset/SHA-256, Linux, macOS) are read from
`tools/mpv/pins.env` by the staging scripts.

## macOS

Disabled unless repo variable `ENABLE_MACOS` is `true` (`gh variable set ENABLE_MACOS --body true`). While it
is off, `mpv-macos`, `build-macos` and the macOS smoke leg are skipped, `publish` still runs, `latest.json`
omits the `darwin-*` keys and no dmg copy is made.

With only `ENABLE_MACOS`, the build is ad-hoc signed (testers run `xattr -dr com.apple.quarantine`).
For the Developer ID path (PLAN §10.5) also set variable `MACOS_DEVELOPER_ID=true` and these secrets:

| Secret | Value |
|---|---|
| `APPLE_CERTIFICATE` | base64 of the exported Developer ID Application `.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | password chosen when exporting the `.p12` |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Your Name (TEAMID)` (`security find-identity -v -p codesigning`) |
| `APPLE_ID` | Apple ID email used for notarisation |
| `APPLE_PASSWORD` | app-specific password from appleid.apple.com |
| `APPLE_TEAM_ID` | 10-character team id |

Export the certificate: in Keychain Access, select the "Developer ID Application" certificate together with its
private key, File -> Export Items -> `.p12`, set a password, then
`base64 -i cert.p12 | gh secret set APPLE_CERTIFICATE` and
`gh secret set APPLE_CERTIFICATE_PASSWORD`. Delete the local `.p12` afterwards.

What the gated steps do: import the cert into a temporary keychain, sign every bundled dylib inside-out with
hardened runtime and timestamp, let `tauri-action` sign the app and notarise with the `APPLE_*` env vars, then
verify with `codesign --verify --deep --strict`, `spctl --assess` and `xcrun stapler validate`. Hardened runtime
will block loading the bundled (differently-signed or ad-hoc) libmpv unless the app has the
`com.apple.security.cs.disable-library-validation` entitlement: add an `entitlements.plist` with that key and
set `bundle.macOS.entitlements` in `tauri.release.macos.conf.json` before enabling this path.
These steps are written but untested.

## Updater keypair

Generate once, locally. Never commit it.

```bash
cargo tauri signer generate -w ~/.tauri/downlink.key
```

- Put the **public** key into `plugins.updater.pubkey` in `tauri.conf.json` (below).
- Store the private key file contents and password as the two secrets above.
- Losing the private key means installed clients can never update; back it up offline.

## Signing notes (PLAN §10.5)

- Updater minisign signatures are mandatory on every platform.
- Windows: unsigned for dev/testers (SmartScreen: More info, Run anyway).
- macOS: off initially; ad-hoc (`signingIdentity: "-"`) when enabled, Developer ID when `MACOS_DEVELOPER_ID` is set (see "macOS"). Testers run `xattr -dr com.apple.quarantine /Applications/Downlink.app`
  once. The smoke job runs `codesign --verify --deep --strict` and checks every `libmpv*.dylib` is
  universal. Moving to a Developer ID later means inside-out signing of every bundled dylib with
  hardened runtime plus notarisation in `build-macos`.
- Linux: `.sig` per artefact plus `SHA256SUMS-linux-x86_64.txt`.

## tauri.conf.json requirements

Coordinator to apply (all paths relative to `apps/desktop/`).

Main `apps/desktop/tauri.conf.json` changes:

```json
{
  "bundle": {
    "active": true,
    "targets": ["nsis", "dmg", "app", "appimage", "deb", "rpm"],
    "createUpdaterArtifacts": true,
    "icon": ["icons/icon.png"],
    "macOS": { "signingIdentity": "-" },
    "windows": { "nsis": { "installMode": "currentUser" } }
  },
  "plugins": {
    "updater": {
      "pubkey": "<contents of downlink.key.pub>",
      "endpoints": ["https://github.com/slamanna212/Downlink/releases/latest/download/latest.json"],
      "windows": { "installMode": "passive" }
    }
  }
}
```

Notes: the dev channel endpoint (`.../releases/download/dev/latest.json`) is chosen at runtime from
Settings -> Updates (updater builder `.endpoints(...)`), not in the file. A real `icon.ico`/`icon.icns`
set is needed for NSIS and dmg (`cargo tauri icon`).

**libmpv bundle paths go in release-only overlay files, not the main config**, so `cargo clippy` and
`cargo tauri dev` do not fail when `tools/mpv/out/` is empty (Tauri checks that resource paths exist at
compile time). The release workflow merges `tauri.release.<os>.conf.json` over the main config:

`apps/desktop/tauri.release.windows.conf.json`

```json
{ "bundle": { "resources": { "../../tools/mpv/out/windows/mpv/": "mpv/" } } }
```

`apps/desktop/tauri.release.macos.conf.json`

```json
{ "bundle": { "macOS": { "files": { "Frameworks": "../../tools/mpv/out/macos/universal" } } } }
```

`apps/desktop/tauri.release.linux.conf.json`

```json
{
  "bundle": {
    "linux": {
      "appimage": { "files": { "usr/lib": "../../tools/mpv/out/linux/lib" } },
      "deb": { "files": { "/usr/lib/downlink/mpv": "../../tools/mpv/out/linux/lib" }, "depends": [] },
      "rpm": { "files": { "/usr/lib/downlink/mpv": "../../tools/mpv/out/linux/lib" } }
    }
  }
}
```

The build fails with a clear message if an overlay file is missing. If directory-valued `files` entries
turn out not to be supported by the bundler version in use, enumerate the libraries instead.
The Rust loader must resolve (bundled -> `DOWNLINK_MPV_PATH` -> system) per PLAN §4.10; for deb/rpm the
private dir is `/usr/lib/downlink/mpv/` (linux.sh already sets `$ORIGIN` rpaths on the staged libs).

## Desktop app requirements (smoke job)

The smoke job needs a `--self-test` flag on the desktop binary. Contract:

- Runs before any window or webview is created; must not need a display server beyond what Xvfb gives.
- Loads libmpv through the normal load order with `DOWNLINK_MPV_PATH` unset, and **fails if the library
  came from anywhere but the bundled location** (so a system libmpv can never mask a packaging bug).
- Creates an mpv handle with `vo=null`, `ao=null`, plays `av://lavfi:testsrc=size=320x240:rate=25` (or a
  tiny embedded clip) and waits for the first decoded frame (`playback-restart`), timeout 30 s.
- Writes a one-line JSON result (`{"ok":true,"libmpv":"<path>","mpv_version":"..."}`) to stdout **and**,
  if `DOWNLINK_SELF_TEST_OUT` is set, to that file (the Windows GUI-subsystem exe has no console).
- Exit code 0 on success, non-zero otherwise. The result must go through `redact/`.

## Open items and assumptions

- The staging scripts take an output dir (`windows.ps1 -OutDir`, `linux.sh <dir>`, `macos.sh <dir> collect|merge`);
  the workflow stages into `tools/mpv/out/{windows/mpv,macos/universal,linux/lib}`. Pins come from `tools/mpv/pins.env`.
- Ubuntu 22.04 apt ships libmpv 0.34 (`libmpv.so.1`), not the 0.40 / `libmpv.so.2` that M0 ran on. Linux
  staging therefore needs a pinned libmpv.so.2 source (`LIBMPV_SRC`) or the pinned build container.
- Dev versions: tag, updater version and app version are all `X.Y.Z-dev.<sha>`. For rpm the Tauri
  bundler (tauri-bundler >= 2.10, checked in its source) converts the semver pre-release to the rpm
  `Version` tag `X.Y.Z~dev.<sha>` itself, so it sorts below `X.Y.Z`; the file name keeps the semver form
  (`Downlink-X.Y.Z-dev.<sha>-1.x86_64.rpm`). Debian accepts the hyphenated version as upstream-revision.
  The workflow installs the latest `tauri-cli ^2`; do not pin it below 2.10. No overlay change is needed.
- Installed binary name is derived at smoke time (`dpkg -L`, `CFBundleExecutable`, NSIS install dir
  `%LOCALAPPDATA%\Downlink`); adjust if `productName` or the install mode changes.
