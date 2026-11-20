# Releasing Downlink

> ## Before the first release: TODO for Sam
>
> **Heads up: pushing to `main` now builds a draft release** (the version in the root `package.json`, currently
> `0.1.0`) if that release does not exist yet. Use `[skip release]` in the commit message to avoid it.
>
> - [ ] Generate the updater keypair (back it up offline; never commit it):
>   `cargo tauri signer generate -w ~/.tauri/downlink.key`
> - [ ] Add the secrets:
>   `gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/downlink.key`
>   then `gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (it prompts for the value).
> - [ ] Paste the public key (`~/.tauri/downlink.key.pub`) into `plugins.updater.pubkey` in `apps/desktop/tauri.conf.json`.
> - [ ] Create the `dev` branch when you want dev builds (`dev-prerelease.yml` checks it out).
> - [ ] (Optional) repo variables: `LINUX_RUNNER` (Linux runner label, default `ubuntu-22.04`), `CI_RUNNER` (runner
>   for trusted PR actors), `LIBMPV_SRC` (Linux libmpv.so.2 path). The Windows libmpv pin lives in
>   `tools/mpv/pins.env`; no `MPV_*` variables are needed.
> - [ ] Apply the config in "tauri.conf.json requirements" below (the `--self-test` flag exists: `apps/desktop/src/selftest.rs`).
> - [ ] macOS is OFF by default (releases start as Windows + Linux). See "macOS" below to turn it on.

Implements PLAN §10. Modelled on the Apogee repo's workflows.

## How releases work

| Workflow | Trigger | Builds | Version | Result |
|---|---|---|---|---|
| `release.yml` | push to `main` (not README.md / `docs/**`, not `[skip release]`) or manual | `main` | root `package.json` `version` | `vX.Y.Z` **draft** release (marked pre-release if the version has a `-suffix`) for you to publish by hand; skipped if the release already exists |
| `dev-prerelease.yml` | manual only ("Run workflow") | `dev` branch | `<package.json base>-dev.<shortsha>` | pre-release `vX.Y.Z-dev.<sha>` published immediately, rolling `dev` release gets the new `latest.json` |
| `pr-ci.yml` | pull requests to `dev` | PR head | none | quality gates only (`ci.yml`) |
| `ci.yml` | called by the above; manual | any | none | fmt, clippy, deny, nextest, UI lint/typecheck/test/build |
| `build.yml` | called by `release.yml` and `dev-prerelease.yml` | any | input | the shared build and publish pipeline |

**The root `package.json` `version` is the single source of truth.** `tauri.conf.json`, the workspace
`Cargo.toml` and `apps/ui/package.json` are overwritten at build time by `tools/release/set-version.mjs`, so
they can never drift (their committed values are irrelevant to releases).

### Cutting a release

```bash
tools/release/bump-version.sh 0.2.0      # edits only the root package.json
git commit -am "chore: release v0.2.0"
git push                                 # on main: release.yml publishes v0.2.0
```

Any push to `main` with an unchanged version is a no-op (release exists). A commit that must not trigger
anything (CI tweaks, etc.) can contain `[skip release]`. Hand-edit the release notes afterwards (git-cliff
output is the starting point; user-visible "What's new" first).

### Pipeline

1. `check-version` (release/dev callers): reads the version, checks the release does not exist, sets the
   pre-release flag. `ci`: the full quality gates must pass.
2. `build.yml`:
   - **prepare**: git-cliff notes, creates a *draft* release, detects the signing key.
   - **mpv-macos** then **build-macos** (only with `ENABLE_MACOS`), **build-windows**, **build-linux**: stage libmpv,
     inject the version, build the UI, `tauri-action` builds and uploads to the draft (`uploadUpdaterJson: false`).
     Linux also signs deb and rpm (Tauri signs only NSIS, AppImage, app.tar.gz) and uploads
     `SHA256SUMS-linux-x86_64.txt`.
   - **publish**: builds `latest.json` with `tools/release/make-latest-json.py` (fails if an asset or `.sig`
     is missing; no darwin keys without macOS), makes unversioned copies (stable non-pre-release only), attests
     provenance. Dev channel: publishes the pre-release and refreshes the rolling `dev` release. Main: leaves the
     release as a draft (as Apogee does) for you to publish from GitHub; `latest.json` is already attached.
   - **smoke**: installs the published artefacts on a clean runner and runs `--self-test` (see below).
   - **cleanup**: if a build or publish failed, a still-draft release and its tag are deleted (as in Apogee).

Differences from Apogee: one reusable `build.yml` instead of duplicated jobs; our own `latest.json`
(parallel `tauri-action` jobs race on one manifest and cannot emit deb/rpm keys).

Linux runner: `build.yml`/`ci.yml` use `vars.LINUX_RUNNER` (default `ubuntu-22.04`). A non-GitHub-hosted runner
skips the apt dependency install, so provision it yourself. The glibc baseline is 22.04.

## Repository secrets and variables

| Name | Kind | Purpose |
|---|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | secret | minisign private key (contents of the key file) for updater `.sig` files |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | secret | its password |
| `ENABLE_MACOS` | variable | `true` turns on the macOS jobs (libmpv, build, smoke) and the darwin keys in `latest.json`; default off |
| `MACOS_DEVELOPER_ID` | variable | `true` switches macOS builds from ad-hoc to Developer ID + notarisation (needs the Apple secrets) |
| `APPLE_*` | secrets | see "macOS" below; only used when `MACOS_DEVELOPER_ID` is `true` |
| `LINUX_RUNNER` | variable (optional) | Linux runner label for build and CI (default `ubuntu-22.04`) |
| `CI_RUNNER` | variable (optional) | runner for PR CI from trusted actors (repo owner, Dependabot); unset = GitHub-hosted |
| `LIBMPV_SRC` | variable (optional) | absolute path of the `libmpv.so.2` to bundle on Linux; default is the runner's linker cache |

`GITHUB_TOKEN` is used for everything else. Workflow permissions are least-privilege per job
(`contents: write`; `id-token` + `attestations: write` for the build/publish call).

## Testing dev builds before the updater key exists

If `TAURI_SIGNING_PRIVATE_KEY` is empty, builds (main and dev) still publish all installers (NSIS `.exe`,
`.deb`, `.AppImage`, `.rpm`, `.dmg` when macOS is on, plus `SHA256SUMS-linux-x86_64.txt`), with a workflow
warning annotation. The build gets `bundle.createUpdaterArtifacts=false`
(merged by `.github/actions/setup-build`), no `.sig` files are made, and `latest.json` and the rolling
`dev` release update are skipped, so in-app updates do not work until the key is added. Libmpv pins (Windows tag/asset/SHA-256, Linux, macOS) are read from
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
