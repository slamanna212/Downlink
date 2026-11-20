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
> - [ ] Paste the public key (`~/.tauri/downlink.key.pub`) into `plugins.updater.pubkey` in `apps/desktop/tauri.conf.json`.
> - [ ] Re-enable the Release workflow on GitHub (it was disabled by hand): `gh workflow enable release.yml`.

## Workflows

| Workflow | Trigger | Result |
|---|---|---|
| `release.yml` | push to `main` **that changes `package.json`** (or manual retry) | Linux tests, then Windows + Linux build into a **draft** release `vX.Y.Z` (version from the root `package.json`). Skipped if that release exists. A `-suffix` version is marked pre-release. |
| `dev-build.yml` | manual only: Actions -> Dev build -> Run workflow, pick the branch (normally `dev`) | Linux tests, then Windows + Linux build, published straight away as pre-release `vX.Y.Z-dev.N`. Points the in-app dev channel at it and keeps only the 5 newest dev builds. |
| `release-notes.yml` | you publish or edit a release | Copies the release text into that release's `latest.json`, so the in-app "What's new" matches GitHub. |
| `ci.yml` | called by the above; manual | Quality gates on Linux only: fmt, clippy, deny, nextest, UI lint/typecheck/test/build. |
| `pr-ci.yml` | pull requests to `dev` | `ci.yml`. |
| `build.yml` | called by `release.yml` and `dev-build.yml` | The shared build and publish pipeline. |

Pushes to `main` that don't touch `package.json` start nothing. Windows is only compiled by the release build
itself (no Windows/macOS test legs).

## Cutting a release

```bash
tools/release/bump-version.sh 0.8.0      # root package.json, synced into Cargo.toml/Cargo.lock/tauri.conf.json/ui
git commit -am "chore: release v0.8.0"
# merge to main and push: release.yml builds the v0.8.0 draft (about 20-30 min)
```

Then on GitHub: open the draft, rewrite the notes (git-cliff's Conventional Commits list is the starting point;
put the user-visible "What's new" first) and press Publish. Publishing runs `release-notes.yml`, which puts your
final text into `latest.json`. Fixing a typo later works the same way: edit the release.

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
| `latest.json` | built by `tools/release/make-latest-json.py` |

Stable releases also get fixed-name copies for website links (`/releases/latest/download/<name>`):
`Downlink_x64-setup.exe`, `Downlink_amd64.AppImage`, `Downlink_amd64.deb`.

Linux builds on `ubuntu-24.04`, the oldest GitHub image with `libmpv.so.2`, so the glibc baseline is 2.39
(Debian 13, Ubuntu 24.04, Fedora 40 and newer). libmpv and its non-system dependencies are bundled by
`tools/mpv/linux.sh`; Windows libmpv is pinned in `tools/mpv/pins.env`.

If a build fails, the half-filled draft and its tag are deleted.

## Secrets and variables

| Name | Kind | Purpose |
|---|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | secret | minisign private key (contents of the key file) for updater `.sig` files |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | secret | its password |
| `LINUX_RUNNER` | variable (optional) | Linux runner label for build and CI (default `ubuntu-24.04`); a self-hosted runner must have the build deps installed |
| `CI_RUNNER` | variable (optional) | runner for PR CI from trusted actors; unset = GitHub-hosted |
| `LIBMPV_SRC` | variable (optional) | absolute path of the `libmpv.so.2` to bundle on Linux |

Without `TAURI_SIGNING_PRIVATE_KEY`, builds still publish the installers (with a warning), but without `.sig`
files or `latest.json`, so in-app updates don't work until the key is added.

## Updater keypair

- The **public** key goes in `plugins.updater.pubkey` in `apps/desktop/tauri.conf.json` (compiled into the app).
- The private key file contents and password are the two secrets above.
- Losing the private key means installed clients can never update again. Back it up offline.

## Not in the pipeline (yet)

- **macOS.** The staging script (`tools/mpv/macos.sh`) and overlay (`apps/desktop/tauri.release.macos.conf.json`)
  are kept. Adding it means a macOS build job (universal libmpv, Developer ID signing and notarisation, PLAN §10.5)
  and passing `--macos` to `make-latest-json.py`.
- rpm, build-provenance attestation, smoke-testing installers on clean runners (`--self-test` still exists in the
  app for manual checks), Windows code signing.
