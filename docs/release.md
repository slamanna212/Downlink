# Releasing Downlink

Implements PLAN §10. Same model as Apogee and MatchExec: a version bump merged to `main` makes a draft release;
dev builds are a button.

> ## Before the first release
>
> - [ ] Generate the updater keypair (back it up offline; never commit it):
>   `cargo tauri signer generate -w ~/.tauri/downlink.key`
> - [ ] Add the secrets:
>   `gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.tauri/downlink.key`
>   then `gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (it prompts for the value).
> - [x] Paste the public key (`~/.tauri/downlink.key.pub`) into `plugins.updater.pubkey` in `apps/desktop/tauri.conf.json`.
> - [x] Re-enable the Release workflow on GitHub (it was disabled by hand): `gh workflow enable release.yml`.
> - [ ] Add the five Apple secrets for a signed, notarised macOS build (see "macOS signing and notarisation").
>   Without them the macOS build is ad-hoc signed, with a warning.

## Workflows

| Workflow | Trigger | Result |
|---|---|---|
| `release.yml` | every push to `main` (or manual retry) | Linux tests, then Windows + Linux + macOS build into a **draft** release `vX.Y.Z` (version from the root `package.json`). Skipped if that release exists. A `-suffix` version is marked pre-release. |
| `dev-build.yml` | manual only: Actions -> Dev build -> Run workflow, pick the branch (normally `dev`) | Linux tests, then Windows + Linux + macOS build, published straight away as pre-release `vX.Y.Z-dev.N`. Points the in-app dev channel at it and keeps only the 5 newest dev builds. |
| `release-notes.yml` | you publish or edit a release | Copies the release text into that release's `latest.json` as an offline fallback. The app itself reads the GitHub release body (see "Release notes in the app"). |
| `ci.yml` | called by the above; manual | Quality gates on Linux only: fmt, clippy, deny, nextest, UI lint/typecheck/test/build. |
| `pr-ci.yml` | pull requests to `dev` | `ci.yml`. |
| `build.yml` | called by `release.yml` and `dev-build.yml` | The shared build and publish pipeline. |

A push to `main` whose version already has a release stops after the version check. Windows is only compiled by the release build
itself (no Windows/macOS test legs).

## Cutting a release

```bash
tools/release/bump-version.sh 0.8.0      # root package.json, synced into Cargo.toml/Cargo.lock/tauri.conf.json/ui
git commit -am "chore: release v0.8.0"
# merge to main and push: release.yml builds the v0.8.0 draft (about 20-30 min)
```

Then on GitHub: open the draft, rewrite the notes (git-cliff's Conventional Commits list is the starting point;
put the user-visible "What's new" first) and press Publish. Fixing a typo later works the same way: edit the release.

### Release notes in the app

The release body on GitHub is what users see under Settings -> Updates -> "What's new". It is Markdown
(GitHub-flavoured: lists, tables, links, **images**), so drag pictures into the release editor and they
show up in the app. The app asks Rust to fetch `GET https://api.github.com/repos/<owner>/<repo>/releases/tags/v<version>`
(unauthenticated, owner/repo taken from the updater endpoint) when an update is available. Edits are live
without touching any artefact; the result is cached for 10 minutes. If GitHub is unreachable, rate-limited
or the body is empty, the app shows the `notes` copied into `latest.json` by `release-notes.yml` instead.
Raw HTML in the body is not rendered, links open in the system browser, and the webview CSP allows images
from `github.com` and `*.githubusercontent.com` only. Drafts have no public API entry, so notes appear once
the release is published (the dev-channel pre-releases are public immediately).

**The root `package.json` `version` is the single source of truth.** `tauri.conf.json`, the workspace
`Cargo.toml` and `apps/ui/package.json` are synced from it by `bump-version.sh` (so local builds report the right
version in the User-Agent and About) and overwritten again at build time by `tools/release/set-version.mjs`.

## Dev builds

Version `X.Y.Z-dev.<run number>`. The run number always goes up, so the updater can order dev builds. `X.Y.Z` is
the `package.json` version if it is ahead of the newest stable release, otherwise the next patch after that
release (stable `0.8.0` -> dev builds `0.8.1-dev.N`), so a dev build never sorts below the release before it.

The rolling `dev-channel` release (not `dev`: that's the branch) holds the newest dev `latest.json`.

## Update channels (in the app: Settings -> Updates)

| Channel | Endpoint |
|---|---|
| stable | `https://github.com/slamanna212/Downlink/releases/latest/download/latest.json` (GitHub's "latest" skips drafts and pre-releases) |
| dev | `https://github.com/slamanna212/Downlink/releases/download/dev-channel/latest.json` |

Checks 60 s after start and every 6 h, downloads in the background, installs on quit. When an update is
available, Settings -> Updates shows its release notes. Switching dev -> stable only offers a build newer than
the installed one.

## Artefacts

| File | Updater key |
|---|---|
| `Downlink_<ver>_x64-setup.exe` + `.sig` | `windows-x86_64` |
| `Downlink_<ver>_amd64.AppImage` + `.sig` | `linux-x86_64` |
| `Downlink_<ver>_amd64.deb` + `.sig` | `linux-x86_64-deb` (Tauri signs only the AppImage; the workflow signs the deb) |
| `Downlink_<ver>_aarch64.dmg` | none (the installer people download) |
| `Downlink_<ver>_aarch64.app.tar.gz` + `.sig` | `darwin-aarch64` (made by the macOS job from the notarised `.app`) |
| `latest.json` | built by `tools/release/make-latest-json.py` |

Stable releases also get fixed-name copies for website links (`/releases/latest/download/<name>`):
`Downlink_x64-setup.exe`, `Downlink_amd64.AppImage`, `Downlink_amd64.deb`, `Downlink_aarch64.dmg`.

Every platform ships the same mpv release, pinned in `tools/mpv/pins.env` (currently 0.42.0). Windows uses
mpv's official libmpv zip. Linux builds libplacebo + mpv from verified source inside the pinned Debian 13
container (`tools/mpv/build-linux.sh`), and the Linux CI and release jobs run in that same container, so the
floor is glibc 2.41 / Wayland 1.23 (Debian 13, Ubuntu 25.04, Fedora 42 and newer). CI caches the libmpv build
keyed on the pins and scripts. Dev machines get the identical bundle with `tools/mpv/linux-docker.sh`, then
`export DOWNLINK_MPV_PATH=$PWD/tools/mpv/out/linux/lib/libmpv.so.2`. macOS builds FFmpeg, libass and
libplacebo from verified source and links them statically into one `libmpv.2.dylib` (`tools/mpv/macos.sh`, run
natively on the arm64 runner), also cached.

If a build fails, the half-filled draft and its tag are deleted.

## Secrets and variables

| Name | Kind | Purpose |
|---|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | secret | minisign private key (contents of the key file) for updater `.sig` files |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | secret | its password |
| `APPLE_CERTIFICATE` | secret | Developer ID Application certificate + private key, exported as `.p12`, base64 |
| `APPLE_CERTIFICATE_PASSWORD` | secret | the `.p12` export password |
| `APPLE_API_ISSUER` | secret | App Store Connect API issuer ID (UUID), for notarisation |
| `APPLE_API_KEY_ID` | secret | App Store Connect API key ID (10 characters) |
| `APPLE_API_PRIVATE_KEY` | secret | contents of that key's `AuthKey_<id>.p8` |
| `LINUX_RUNNER` | variable (optional) | Linux runner label for build and CI (default `ubuntu-24.04`); the Linux jobs run in a container, so a self-hosted runner needs Docker |
| `CI_RUNNER` | variable (optional) | runner for PR CI from trusted actors; unset = GitHub-hosted |

Without `TAURI_SIGNING_PRIVATE_KEY`, builds still publish the installers (with a warning), but without `.sig`
files or `latest.json`, so in-app updates don't work until the key is added.

## Updater keypair

- The **public** key goes in `plugins.updater.pubkey` in `apps/desktop/tauri.conf.json` (compiled into the app).
- The private key file contents and password are the two secrets above.
- Losing the private key means installed clients can never update again. Back it up offline.

## macOS signing and notarisation

Apple Silicon only (macOS 13+); Intel Macs are not built. The `build-macos` job in `build.yml`, on `macos-15`:

1. Restores or builds `tools/mpv/out/macos` (`tools/mpv/macos.sh`).
2. Imports `APPLE_CERTIFICATE` into a throwaway keychain and finds the `Developer ID Application: …` identity in it
   (so the identity name is not a secret of its own).
3. Signs `libmpv.2.dylib` with that identity, hardened runtime and a secure timestamp.
4. Builds with `bundle.macOS.signingIdentity` set to the identity; tauri signs the app (hardened runtime,
   `apps/desktop/entitlements.plist`), notarises it with the App Store Connect API key and staples the ticket.
5. Verifies: `codesign --verify --deep --strict`, `stapler validate`, `spctl --assess`, then runs the bundled app's
   `--self-test` (loads the bundled libmpv under the hardened runtime and plays a test pattern).
6. Uploads the dmg and `Downlink_<ver>_aarch64.app.tar.gz`; publish signs the archive for the updater.

All five `APPLE_*` secrets or none: none gives an ad-hoc signed, un-notarised build with a warning (testers run
`xattr -dr com.apple.quarantine /Applications/Downlink.app` once); a partial set fails the job.

Renewing: the Developer ID certificate lasts 5 years. Builds signed before it expires keep working (the timestamp
proves when they were signed). Make a new one, re-export the `.p12` and replace `APPLE_CERTIFICATE` and
`APPLE_CERTIFICATE_PASSWORD`. If the API key is revoked, make a new one and replace the three `APPLE_API_*` secrets.

## Not in the pipeline (yet)

- macOS on Intel (would need an x86_64 libmpv and a universal build), notarising the dmg itself (the app inside is
  notarised and stapled, which is what Gatekeeper checks).
- rpm, build-provenance attestation, smoke-testing installers on clean runners (`--self-test` still exists in the
  app for manual checks), Windows code signing.
