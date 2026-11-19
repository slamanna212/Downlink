# Dev setup

Everything here is synthetic. Automated tests never hit a real server (CLAUDE.md rule 16); the local
Dispatcharr only ever sees the fixture playlist.

## Toolchain (PLAN §13.2)

```bash
rustup toolchain install stable && rustup component add clippy rustfmt
corepack enable && corepack prepare pnpm@10 --activate
cargo install tauri-cli --version "^2" --locked
cargo install cargo-nextest cargo-deny git-cliff --locked

# Linux build deps (Ubuntu 24.04 dev box)
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
  librsvg2-dev libmpv-dev ffmpeg pkg-config build-essential
```

Also needed for this environment: docker (run as your user), `ffmpeg`/`ffprobe` 6+ with libx264/libx265,
python 3.10+ (stdlib only). Local dev may use a system libmpv via `DOWNLINK_MPV_PATH`; releases never do.

## Bring up the test environment

```bash
tools/dev/up.sh      # stream server + Dispatcharr + bootstrap (idempotent, safe to re-run)
tools/dev/down.sh    # stop both; data kept
```

What it starts:

| Piece | Where | Script |
|---|---|---|
| Synthetic TS channels, M3U, XMLTV | `http://localhost:8088` (bound on 0.0.0.0 so the container can reach it) | `tools/fixtures/streams/streams-up.sh` / `streams-down.sh`; log in `tools/dev/.state/streams.log` |
| Test Dispatcharr 0.34.1 (pinned image tag in `dispatcharr-up.sh`) | `http://localhost:9191`, container `dispatcharr-test`, volume `dispatcharr_test`, `--add-host=host.docker.internal:host-gateway` | `tools/dev/dispatcharr-up.sh` / `dispatcharr-down.sh [--purge]` |
| Bootstrap via Dispatcharr's own REST API | admin user, API key, M3U account, XMLTV source, 10 channels with EPG, catch-up on `Catchup One`, Redirect profile on `Redirect One` | `tools/dev/bootstrap-dispatcharr.py` |

Credentials and the API key are written to **`tools/dev/.env.local`** (gitignored, mode 600):
`DISPATCHARR_URL`, `DISPATCHARR_USER`, `DISPATCHARR_PASSWORD`, `DISPATCHARR_API_KEY`. Use it:

```bash
set -a; . tools/dev/.env.local; set +a
curl -H "X-API-Key: $DISPATCHARR_API_KEY" "$DISPATCHARR_URL/api/channels/channels/?page=1&page_size=500"
cargo run -p downlink-cli -- --source $DISPATCHARR_URL --key $DISPATCHARR_API_KEY channels --search news
```

Reset from scratch: `tools/dev/dispatcharr-down.sh --purge && tools/dev/up.sh`.
Bump Dispatcharr: edit `IMAGE` in `dispatcharr-up.sh`, purge, up, then re-run
`python3 tools/dev/capture-fixtures.py` and diff `tools/fixtures/dispatcharr/`; update
`docs/dispatcharr-endpoints.md`.

Docker to host networking: Dispatcharr fetches the playlist and the EPG as `host.docker.internal:8088`.
The Redirect channel's stream URL uses the docker bridge gateway (`172.17.0.1`) so the 302 `Location` is also
reachable from the host.

## Synthetic media and channels

```bash
tools/fixtures/streams/make-media.sh [--force]   # writes tools/fixtures/media/*.ts (gitignored, ~20 s clips)
tools/fixtures/streams/probe-media.sh            # ffprobe summary
```

| Clip | Used by |
|---|---|
| `h264_aac_1080p30` | Channel One, Catchup One, Redirect One, Flaky Drop, Test Movies, Test Kids |
| `hevc_ac3_1080p30` | Test News |
| `h264_1080i_tff` | Test Sports 1080i |
| `broken_timestamps` (PTS/DTS jitter, backwards jump, forward gap) | Broken Clock (raw byte-loop mode, so the discontinuities survive) |
| `hevc_hdr10` (Main 10, BT.2020/PQ, mastering metadata) | HDR Demo |

`Flaky Drop` closes the connection 15 s into each request. Channel definitions: `tools/fixtures/streams/channels.json`.
The server loops clips with `ffmpeg -re -stream_loop -1 -c copy` (continuous timestamps), except raw-mode channels.
Playlist: `/playlist.m3u?streamhost=HOST:PORT`; XMLTV: `/epg.xml` (hourly programmes from 60 h ago to 48 h ahead,
generated at request time, synthetic titles). Playlist and EPG URLs can be added directly as an M3U / XMLTV source in
the app, with no Dispatcharr in between.

Direct playback check without Dispatcharr: `mpv http://localhost:8088/ch/channel-one.ts`.

## Fixtures

| Path | Content |
|---|---|
| `tools/fixtures/dispatcharr/*.json` | Scrubbed responses captured from the 0.34.1 container (canonical for wiremock); hosts replaced by `dispatcharr.test`, keys/tokens redacted. Regenerate with `tools/dev/capture-fixtures.py`. |
| `tools/fixtures/xtream/*.json` | `player_api.php` responses: login, live categories/streams, VOD, series, series info, short EPG (base64 titles), auth failure; includes null/string/missing-field quirks |
| `tools/fixtures/m3u/*.m3u` | basic, `#EXTVLCOPT`/`#KODIPROP`/`#EXTGRP`/pipe headers/credentials in URLs, catch-up modes, malformed edge cases. `gen-large-m3u.py [count]` writes a 20k-entry list to `m3u/generated/` (gitignored). |
| `tools/fixtures/xmltv/*` | timezone offset variants, edge cases (entities, CDATA, missing/zero/negative durations, overlaps, duplicates, unknown channels), Latin-1, no header, truncated, gzip |
| `tools/fixtures/gen-edge-fixtures.py` | deterministic generator for the committed xtream/m3u/xmltv files |

## Network discipline (CLAUDE.md rule 4)

The scripts never request a stream endpoint. One explicit manual verification was done while writing
`docs/dispatcharr-endpoints.md`: a ~8 s GET of `/proxy/ts/stream/<channel-one uuid>` (to capture `/proxy/ts/status`) and a
no-follow GET of the Redirect channel (to see the 302). Repeat it only by hand:

```bash
curl -s --max-redirs 0 -i -H "X-API-Key: $DISPATCHARR_API_KEY" "$DISPATCHARR_URL/proxy/ts/stream/<uuid>"
```
