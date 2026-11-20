# Dispatcharr endpoints used by Downlink

Observed against **Dispatcharr 0.34.1** (`ghcr.io/dispatcharr/dispatcharr:0.34.1`, 252 schema paths) running
from `tools/dev/` with synthetic content only. Raw scrubbed responses are in
`tools/fixtures/dispatcharr/<name>.json` (names in brackets below). PLAN §3.4 is the feature map; this file
records what the server actually does. Re-capture with `tools/dev/capture-fixtures.py` after bumping the image.

Auth for every call: `X-API-Key: <key>` (401 with a missing or wrong key). JWT: `POST /api/accounts/token/`
`{username,password}` → `{access, refresh}` [`accounts_token`], `POST /api/accounts/token/refresh/` `{refresh}` →
`{access}`. Only the shape is recorded (values are placeholders).

## Differences from the plan (read first)

| # | Plan said | Observed |
|---|---|---|
| 1 | `GET /proxy/ts/status/{channel_id}` | Takes the **channel UUID**. A numeric channel id returns 404 `{"error":"Channel 1 not found"}`. |
| 2 | Lists are DRF-paginated `{count,next,previous,results}` | Only when `page` or `page_size` is present. `GET /api/channels/channels/` and `…?search=x` with **no** page params return a **bare array** of every match. Always send `page` + `page_size`. `page_size` is honoured up to at least 500 (100000 returned all 10 rows). `next` is an absolute URL built from the request Host header (never trust its host). |
| 3 | Redirect profile "does not open a provider connection" (§3.5) | Before answering 302, Dispatcharr runs `validate_stream_url`: a server-side **HEAD** to the provider URL (redirects followed), and if that is not 2xx a **GET with `stream=True`** (reads the first chunk). So a Redirect channel play costs the provider a HEAD, and sometimes a short GET. Observed in the stream-server log: one `HEAD /ch/redirect-one.ts` per play request. The 302 `Location` is the original profile URL, no credentials, no body. PLAN §3.5 step 2 and the "M0 verifies" note should be reworded. |
| 4 | Effective stream profile at sync | `effective_stream_profile_id` is **null** for a channel on the server default; it is not resolved server-side. Resolve it client-side: `channel.stream_profile_id ?? stream_settings.default_stream_profile` (see below). |
| 5 | Catch-up from M3U tags | Dispatcharr's M3U ingest reads only `tv_archive="1"` and `tv_archive_duration="N"` for `is_catchup`/`catchup_days` (XC accounts: `tv_archive`, `tv_archive_duration` from `get_live_streams`). `catchup="default"` / `catchup-days` / `catchup-source` attributes on a plain M3U are **ignored**. The channel's `is_catchup` is rolled up from its active streams. **Playback additionally requires an XC-type M3U account**: a plain-M3U catch-up stream gets a 201 session but `GET playback_url` returns 400 `Cannot build timeshift URL` (source: `apps/timeshift/views.py::_prepare_catchup_stream_attempt` skips any stream whose account is not `account_type == "XC"` or whose `custom_properties.stream_id` is missing). |
| 6 | `POST …/{id}/position/` `{…}` | Body is `{"position_secs": <number>}`. Returns 404 `{"error":"No active playback for this session"}` until the player has made its first GET on `playback_url`; it cannot be called right after create. |
| 10 | Recording state | `custom_properties.status` is absent for scheduled one-off/series recordings; extend takes `extra_minutes`; recurring rules need both dates and use 0 = Monday (see DVR). |
| 7 | Programme ids are strings | Observed integers for XMLTV EPG in grid, current-programs and search. Keep the string-tolerant decode (dummy EPG ids may still be strings). |
| 8 | Grid associates programmes to channels | `/api/epg/grid/` rows carry **only `tvg_id`** (no channel id/uuid). Several channels sharing one `tvg_id` receive the same rows; join on `tvg_id` client-side. |
| 9 | Catch-up `playback_url` | Relative path `/proxy/catchup/<channel_uuid>?session_id=<id>`; prefix the server base URL. That path is a stream endpoint (rule 4): only request it on an actual play. |

## Connect sequence

| Call | Notes |
|---|---|
| `GET /api/core/version/` (no auth needed) | `{"version":"0.34.1","timestamp":null}` [`core_version`] |
| `GET /api/accounts/users/me/` | `{id, username, api_key, email, user_level, channel_profiles[], custom_properties, avatar_config, stream_limit, is_staff, is_superuser, last_login, date_joined, first_name, last_name}` [`users_me`]. `user_level` 10 = admin. **`api_key` is the full secret** — never log or store it outside the keychain. |
| `GET /api/core/timezones/` | `{"timezones":[…IANA names…]}` [`core_timezones`] |
| `GET /api/core/settings/` | Bare array of `{id,key,name,value}`. Relevant keys: `stream_settings`, `system_settings` [`core_settings`]. Admin-only content may be hidden from lower levels; treat a 403 as "unknown". |

First-run bootstrap (dev only, not a Downlink feature): `GET/POST /api/accounts/initialize-superuser/`
(`{username,password}`; IP-gated, closes once an admin exists), `POST /api/accounts/api-keys/generate/` → `{key,user}`.

## Stream profiles and the server default (resolved)

- List: `GET /api/core/streamprofiles/` → bare array `{id,name,command,parameters,is_active,user_agent,locked}`
  [`core_streamprofiles`]. Built-ins on 0.34.1: `1 FFmpeg`, `2 Streamlink`, `3 Proxy`, `4 Redirect`, `5 VLC`.
  Redirect is identified by `name == "Redirect"` and `locked == true` (empty `command`); there is no `type` field.
- Server default: `GET /api/core/settings/` → entry `key == "stream_settings"` →
  `value.default_stream_profile` (profile **id**; `1` = FFmpeg on a fresh install). Same entry has
  `m3u_hash_key`, `default_user_agent`.
- A channel's profile: `stream_profile_id` (nullable), else the default above. Streams also carry
  `stream_profile_id`. Note that "Proxy" and "FFmpeg" both go through `/proxy/ts/stream/`; only `Redirect`
  answers 302.

## Channels

| Call | Params | Response |
|---|---|---|
| `GET /api/channels/channels/` | `page`, `page_size`, `search`, `channel_group` (group **name**, exact), `name`, `epg`, `ordering` | paginated envelope plus extra `has_unassigned_epg_channels` [`channels_page`, `channels_by_group`]; bare array when no page params [`channels_search_unpaginated`] |
| `GET /api/channels/channels/{id}/streams/` | — | bare array of `Stream`, failover order [`channel_streams`, `channel_streams_catchup`] |
| `GET /api/channels/groups/` | — | bare array `{id,name,channel_count,is_custom,m3u_account_count,m3u_accounts[{channel_group,enabled,auto_channel_sync,…,is_stale,last_seen,stream_count}]}`; includes empty groups such as "Default Group" with `channel_count: 0` [`channels_groups`] |
| `GET /api/channels/profiles/` | — | bare array (empty on a fresh server) [`channels_profiles`] |
| `GET /api/channels/logos/` | `page`, `page_size` | paginated, empty on the synthetic server [`channels_logos`]; `GET /api/channels/logos/{id}/cache/` → 404 for an unknown id (no logos exist locally, image bytes not captured) |

`Channel` fields as observed [`channel_redirect`]: `id, channel_number (float), name, channel_group_id, tvg_id,
tvc_guide_stationid, epg_data_id, streams[] (stream ids), stream_profile_id, uuid, logo_id, user_level, is_adult,
is_catchup, catchup_days, is_radio, hidden_from_output, auto_created, auto_created_by, auto_created_by_name,
override, source_stream` plus `effective_name, effective_channel_number, effective_channel_group_id,
effective_logo_id, effective_tvg_id, effective_tvc_guide_stationid, effective_epg_data_id,
effective_stream_profile_id, effective_is_radio`. Prefer the `effective_*` fields for display and grouping
(they apply per-channel overrides).

`Stream` adds `url` (provider URL, **do not log**), `m3u_account`, `stream_stats{resolution, source_fps, video_codec,
audio_codec, audio_channels, pixel_format, …}`, `stream_id` (XC id, null for plain M3U), `stream_chno`,
`is_catchup`, `catchup_days`, `is_stale`, `current_viewers`.

## Guide and programmes

| Call | Params | Response |
|---|---|---|
| `GET /api/epg/grid/` | `start`, `end` (ISO 8601, win over days), `prev_days` (0-30), `days` (1-365), `channel_profile_id` | `{"data":[{id,start_time,end_time,title,sub_title,description,tvg_id,season,episode,is_new,is_live,is_premiere,is_finale}]}` flat, UTC `Z` strings [`epg_grid_default`, `epg_grid_window`, `epg_grid_prev_days`]. No params = previous hour to next 24 h. `prev_days=2` reached 48 h back. |
| `POST /api/epg/current-programs/` | body `{channel_uuids:[…]}` (also `epg_data_ids:[…]`; `{}` = all channels with a current programme) | **bare array** of grid rows plus `channel_uuid` [`epg_current_programs_uuids`, `epg_current_programs_all`]. Channels with no current programme are omitted. Not a `{"data":…}` envelope. |
| `GET /api/epg/programs/search/` | see below | paginated envelope [`epg_programs_search_*`] |

**Programme search (resolved).** Filters: `title`, `description` (AND/OR, quoted phrases, parentheses;
`title_regex`, `title_whole_words`, `description_regex`, `description_whole_words`), `channel` (name substring),
`channel_id` (exact id), `tvg_id` (exact), `group` (channel or stream group substring), `stream` (stream name
substring), `epg_source` (id), `airing_at` (instant, `start<=t<end`), `start_after`, `start_before`, `end_after`,
`end_before` (ISO 8601), `fields` (comma list), `page`, `page_size` (default 50, max 500). Result row:
`{id,title,sub_title,description,start_time,end_time,tvg_id,custom_properties{new,season,episode,categories[],onscreen_episode},
epg_source,epg_name,epg_icon_url,channels[{id,name,channel_number,channel_group,tvg_id}],streams[…]}`. Downlink
"search the guide" = `title=<q>` with `start_after=<now>`; "what is on now" = `airing_at=<now>`.

## Catch-up

| Call | Request | Response |
|---|---|---|
| `POST /api/catchup/sessions/` | `{channel_uuid, start, duration?}`; `start` ISO 8601 `Z`, epoch seconds or XC wall clock; `duration` 1-480 minutes | **201** `{session_id, playback_url, expires_at (unix, handshake deadline), channel_uuid, start, duration}` [`catchup_session_create`]. No provider request is made at create time. |
| 400 | channel without catch-up | `{"error":"Catch-up not supported for this channel"}` [`catchup_session_unsupported_channel`]; also 400 for an invalid `start`, 403 when catch-up is disabled for the user/system, 404 for an unknown or inaccessible channel, 503 when the session service is down |
| `POST /api/catchup/sessions/{id}/position/` | `{position_secs}` | 404 until playback has started [`catchup_session_position_no_playback`] |
| `DELETE /api/catchup/sessions/{id}/` | — | 204 (404 for a missing or foreign session) |

**What playback needs (resolved).** An XC account (`account_type: "XC"`, `username`, `password`, `server_url`) whose
`get_live_streams` marks the stream `tv_archive: 1` with `tv_archive_duration`, and a provider that serves
`GET {server}/timeshift/{user}/{pass}/{minutes}/{YYYY-MM-DD:HH-MM}/{stream_id}.ts` (Dispatcharr tries this PATH layout
first; the query form `/streaming/timeshift.php?username&password&stream&start&duration` is used when the client came in
via `timeshift.php`). The start is converted to the provider's timezone from the default profile's `server_info.timezone`
(returned by the XC login), and `minutes` = requested duration plus a ~5 minute lag buffer (60 requested, 65 sent). The
local synthetic server implements all of this (see dev-setup). A verified run: session create 201, then
`ffprobe` of the `playback_url` returned h264/aac 1080p and the provider log showed
`/timeshift/testuser/testpass/65/2026-11-18:17-00/101.ts`.

Catch-up is on by default (`system_settings.catchup_enabled` unset = true; per-user
`custom_properties.catchup_enabled == false` disables it). Sessions expire if the first playback GET does not
happen within the handshake window, and idle after 10 minutes.

## Live playback and my session (stream endpoints: play only)

| Call | Notes |
|---|---|
| `GET /proxy/ts/stream/{channel_uuid}` | TS stream, or 302 for Redirect-profile channels (difference 3). |
| `GET /proxy/ts/status` | `{"channels":[…],"count":N}` [`proxy_ts_status_idle`, `proxy_ts_status_playing`]. Status endpoint, safe to poll. |
| `GET /proxy/ts/status/{channel_uuid}` | Single object, same fields plus `m3u_profile_name`, `state_changed_at`, `state_duration`, `total_data` [`proxy_ts_status_channel`]; 404 `{"error":"Channel … not found"}` when nothing is playing. |
| `GET /proxy/ts/stop/{channel_id}`, `/stop_client/{channel_id}` | per the schema; see TS lifecycle in PLAN §3.5. |
| `DELETE /api/proxy/hls/sessions/{token}/` | 204, no list endpoint. |

`/proxy/ts/status` channel object (live example, one client):
`channel_id (uuid), state ("active"), url (provider URL!), stream_profile (id as string), owner, buffer_index,
client_count, uptime (s), started_at (epoch), channel_name, m3u_profile_id, stream_id, stream_name, total_bytes,
avg_bitrate_kbps, avg_bitrate ("1.97 Mbps"), clients[{client_id, user_agent, output_format, ip_address,
connected_at, user_id (string), output_profile_id}], video_codec, resolution, source_fps, ffmpeg_speed,
audio_codec, audio_channels, audio_streams[{index, audio_codec, audio_channels, audio_bitrate, sample_rate}],
stream_type`. It lists **all** viewers' clients, not only ours; filter `clients[].user_id` / `client_id`. `url`
exposes the provider URL, so it must pass through `redact/` before logging or showing in the debug panel. The
channel disappears from the list within seconds of the last client disconnecting.

## Live HLS (researched in the 0.34.1 source, tried once)

PLAN §3.5 sketches `/proxy/hls/<token>/index.m3u8` as a fallback. What the server actually does:

**There is no create endpoint and the client does not choose the token.** HLS is an *output format* of the normal live endpoint:
`GET /proxy/ts/stream/{channel_uuid}?output_format=hls` (alias `?output=hls`; `m3u8` also accepted; other values `ts|mpegts`, `fmp4|mp4`)
with the usual `X-API-Key`. The server starts a segmenter on the channel's existing buffer, mints a 24-byte url-safe token and answers
`302 Location: /proxy/hls/<token>/index.m3u8` with the token also in response header `X-Dispatcharr-Session-Token`. The Xtream route
`/live/<u>/<p>/<id>.m3u8` forces HLS the same way (`.ts` = mpegts, `.mp4` = fmp4).

What selects the format when the client says nothing: user `custom_properties.output_format`, else `stream_settings.default_output_format`
(unset locally, so `mpegts`). So "the channel's output requires HLS" is **not per channel and not a stream profile**: it is a per-user or
per-server default, or the client's explicit parameter. The stream profile (FFmpeg/Proxy/Redirect) governs how the server ingests the
provider, not what it sends the client. (A Redirect-profile channel is answered by `_handle_redirect_stream` before this logic; combining it
with `output_format=hls` was not tried.)

| Step | Call | Auth | Observed |
|---|---|---|---|
| Mint (starts the session, counts as a play, also opens the provider connection if the channel is cold) | `GET /proxy/ts/stream/{uuid}?output_format=hls` | `X-API-Key` (network access `STREAMS` checked here only) | `302`, `Location: /proxy/hls/<token>/index.m3u8`, `X-Dispatcharr-Session-Token: <token>` |
| Playlist | `GET /proxy/hls/{token}/index.m3u8` | **none: the token is the credential** | `200`, `#EXT-X-VERSION:3`, `TARGETDURATION:6`, 2 s segments here (`#EXTINF:2.000`), live sliding window, relative segment URIs `1.ts`, `2.ts`. Cold channel: the first playlist request is held open (an `#EXTM3U` first line and comment keep-alives, up to ~10 s of media-gate wait) instead of returning an empty list. Warm channel: 5 ms. |
| Segment | `GET /proxy/hls/{token}/{seq}.ts` | none | `200`, 1.4 MB for 2 s of 1080p |
| Stop | `DELETE /api/proxy/hls/sessions/{token}/` | `X-API-Key` (401 without), only the minting user | `204`; afterwards the playlist returns `410`, a second delete `404` |
| List | none | | no listing endpoint. `GET /proxy/ts/status` shows the client with `output_format: "hls"` (and `clients[].client_id`), so our own sessions can be found there, but the token is not exposed |

A minted-but-unused session is cheap to leak but real: it registers a client on the channel. It is reaped by the ghost reaper when no playlist or
segment poll arrives for `HLS_SEGMENT_DURATION x HLS_CLIENT_GHOST_SEGMENTS` = 4 s x 3 = **12 s** (code constants, not user settings); the Redis
client record TTL is 60 s; the cold-start wait is 10 s and if the held first request is dropped the client is stopped after a short grace. My
test minted twice by accident and both were gone after an explicit `DELETE` (the second one was deleted by token; the first by token from the
`Location` header).

Can TS fail in a way HLS fixes? **Mostly no.** Both outputs read the same channel buffer fed by one upstream connection, so a provider stall,
failover or the 60 s `channel_init_grace_period` hits both identically. HLS helps only with the *client-to-Dispatcharr leg*: an intermediary
(reverse proxy, corporate or hotel proxy, CDN) that buffers, times out or breaks a long-lived chunked `video/mp2t` response, because HLS is plain
short GETs that survive reconnects and need no auth header on the polling requests. Costs: latency of roughly 2 to 3 segments (about 6 to 10 s)
on top of TS, a fixed small window (poor for catch-up or timeshift-like seeking), an extra remux step on the server, and a session to clean up.

**Recommendation: do not implement automatic TS-to-HLS fallback for 1.0; document it as unnecessary and keep it as an M5+ opt-in.** libmpv with the
retry/failover policy already covers upstream stalls (which HLS would not fix), and an automatic switch would add a second provider-visible play path
(rule 4: every extra mint is a real play and a client registration). If a user reports a proxy that breaks TS, add a per-source setting "Use HLS transport"
with exactly this flow, all in Rust, no mpv options outside `downlink-player`:

1. At play time only, `GET {base}/proxy/ts/stream/{uuid}?output_format=hls` with `X-API-Key`, redirects disabled (same resolver as the Redirect-profile path in
   PLAN §3.5). Read the token from `X-Dispatcharr-Session-Token` (fall back to parsing `Location`); a non-302 is a normal play error.
2. Hand mpv `{base}/proxy/hls/{token}/index.m3u8` **with no auth headers** (the token is the capability, so the API key never reaches mpv at all).
3. Persist `token` in `open_session` for crash cleanup (CLAUDE.md rule 6); on stop, zap, navigate-away, sleep and quit call `DELETE /api/proxy/hls/sessions/{token}/`
   with the key; treat 404 as already gone.
4. Redaction: `/proxy/hls/<token>/…` is a new credential-bearing URL shape (the token grants playback); add it to `redact/` with a test (rule 8).
5. Expect 410 on playlist or segment as "session ended" (reaped or stopped), and re-mint at most once before surfacing an error.

## VOD (verified against the synthetic Xtream VOD account)

Dispatcharr ingests VOD only from **XC accounts with `enable_vod: true`** (`PATCH /api/m3u/accounts/{id}/ {"enable_vod":true}`, then
`POST /api/m3u/accounts/{id}/refresh-vod/`; ingest is async and took seconds locally). It calls the provider's
`get_vod_categories`, `get_series_categories`, `get_vod_streams`, `get_series` (all in one call each). New categories are
enabled automatically (`auto_enable_new_groups_vod/series`). Fixtures: `vod_*.json`.

| Call | Observed |
|---|---|
| `GET /api/vod/movies/` | paginated. Row: `{id, logo{id,name,url,cache_url,movie_count,series_count,is_used,item_names[]}, custom_properties (null until provider-info), uuid, name, description, year, rating (string), genre, duration_secs, tmdb_id, imdb_id, language, is_adult, created_at, updated_at}`. **Before the first `provider-info` call** `description`, `genre`, `duration_secs` are empty/null; the list only has what `get_vod_streams` carried. `logo.cache_url` is an absolute URL built from the request host (use the path only) |
| `GET /api/vod/movies/{id}/` | same row shape |
| `GET /api/vod/movies/{id}/providers/` | array of `{id, movie, category{id,name,category_type,m3u_accounts[]}, m3u_account{… the full account incl. username/password and profiles …}, stream_id, container_extension, …}` — **embeds provider credentials; never log it, and drop `m3u_account` when decoding** |
| `GET /api/vod/movies/{id}/provider-info/` | **triggers a provider `get_vod_info`** and returns `{id, uuid, stream_id, name, o_name, description, plot, year, release_date, genre, director, actors, country, rating, tmdb_id, imdb_id, youtube_trailer, duration_secs, age, backdrop_path[], cover, cover_big, movie_image, bitrate, video{}, audio{}, container_extension, direct_source, category_id, added, m3u_account{id,name,account_type}}`; image URLs are rewritten to `/api/vod/movies/{id}/image/?kind=…` |
| `GET /api/vod/series/` | paginated; row adds `episode_count` (0 until episodes are fetched) and `custom_properties{cast,director,category_id,releaseDate,backdrop_path[],episode_run_time}` |
| `GET /api/vod/series/{id}/provider-info/` | **triggers `get_series_info` and populates episodes** |
| `GET /api/vod/series/{id}/episodes/` | **bare array** (not paginated). Empty until `provider-info` has run once. Each element embeds `series{…}` and `providers[{… m3u_account with credentials …}]` |
| `GET /api/vod/episodes/` | paginated, same embedded objects |
| `GET /api/vod/categories/` | bare array `{id,name,category_type,category_type_display,m3u_accounts[{category,m3u_account,enabled,custom_properties}]}` (movie and series categories in one list) |
| `GET /api/vod/all/` | paginated mixed list `{id, uuid, name, description, year, rating (number), genre, duration, created_at, updated_at, custom_properties (JSON string!)}` |
| `GET /api/vod/{movies,series,episodes}/{id}/image/` | 200 `image/png` (served via Dispatcharr's image proxy); `GET /api/vod/vodlogos/{id}/cache/` likewise |
| `GET /proxy/vod/movie/{uuid}`, `/proxy/vod/episode/{uuid}` | **stream endpoint.** First answers `301` to `/proxy/vod/movie/{uuid}/{session_id}` (`vod_<ms>_<n>`, `Cache-Control: private, max-age=21600`). The session URL honours `Range` (206) and plays; ffprobe (which follows redirects) opened an episode through it. The session id is created by that first GET, so treat it like the other sessions (stop on leaving). `GET /proxy/vod/stats/` lists `vod_connections[{content_type, content_name, content_uuid, content_metadata, connection_count, connections[]}]`; `POST /proxy/vod/stop_client/` ends one |

Pitfalls found: series `episode_count` and episode lists are lazy (call `provider-info` first); `rating` is a string on
movie/series rows but a number in `/vod/all/`; the embedded `m3u_account` objects include provider credentials, so a typed
decoder should not model them at all.

## DVR (verified locally; fixtures `recording*.json`, `recurring_rules.json`, `series_rule*.json`)

**Recording** (`GET/POST /api/channels/recordings/`, list is a **bare array**):
`{id, start_time, end_time, task_id, custom_properties, channel (int id)}`. Create body: `{channel, start_time, end_time, custom_properties:{program:{title,description}}}`.
- `end_time` must be in the future: past windows get 400 `{"non_field_errors":["End time must be in the future."]}` (`recording_past_error`). `start_time` may be in the past (starts immediately; `task_id` becomes `dvr-now-<id>-<hash>`, otherwise `dvr-recording-<id>`).
- **State is `custom_properties.status`**, and it is **absent while scheduled** for one-off and series-created recordings (`recording_scheduled_oneoff`, `recording_scheduled_series`); recurring-rule recordings carry `status: "scheduled"` (`recording_scheduled_recurring`). Values seen: `scheduled`, `recording`, `completed`, `stopped`; source also has `interrupted` and `cancelled`. Treat missing as scheduled, and finished = `completed|stopped|interrupted` (the same set `extend` refuses).
- **In progress** (`recording_in_progress`): `status: "recording"`, `file_url` = `output_file_url` = `/api/channels/recordings/{id}/hls/index.m3u8`, plus `_hls_dir`, `file_name`, `file_path`, `started_at` (naive UTC string). The playlist is `application/x-mpegURL`, `EVENT` type, 4 s segments, **segment URIs are absolute and built from the request Host** (`recording_in_progress_hls_playlist`): rewrite their origin to the configured server. Segments (`/hls/seg_00000.ts`) return 403 without `X-API-Key`, 200 with, so the player needs the header per file. `GET …/file/` while recording answers `302` to the HLS playlist.
- **Completed/stopped** (`recording_completed`, `recording_after_stop`): `file_url` = `output_file_url` = `/api/channels/recordings/{id}/file/` (Matroska, `Range` supported, `Content-Disposition: inline; filename=…mkv`), plus `ended_at`, `bytes_written`, `remux_success`, `stream_info{width,height,resolution,source_fps,video_codec,audio_codec,…}` when probed. `stopped` adds `stopped_at`; the HLS fields are gone.
- **Extend**: `POST …/{id}/extend/` body is **`{"extra_minutes": n}`** (positive int), not `minutes` (that returns 400 `{"success":false,"error":"extra_minutes must be a positive integer"}`). Success: `200 {"success":true,"new_end_time":"…+00:00"}`; a finished recording returns 400 `{"success":false,"error":"Recording has already finished"}`.
- **Stop**: `POST …/{id}/stop/` → `200 {"success":true,"status":"stopped"}`; partial content stays playable (status becomes `stopped` within seconds, file at `/file/`).
- `DELETE …/{id}/` → 204.

**Recurring rule** (`GET/POST /api/channels/recurring-rules/`): `{id, days_of_week:[int], start_time:"HH:MM:SS", end_time, enabled, name, start_date, end_date, created_at, updated_at, channel}`.
**`start_date` and `end_date` are both required** (400 `Start date is required`, then `End date is required`; the schema marks them nullable but the serializer rejects null). **Day numbering is Python `weekday()`: 0 = Monday … 6 = Sunday** (rule `[0,2,4]` produced recordings on Mon 2026-11-23, Wed 11-25, Fri 11-20). Times are interpreted in the **system time zone** (`system_settings.time_zone`, UTC locally), an overnight window (end <= start) rolls to the next day. Creating a rule materialises recordings immediately for the next 14 days; each carries `custom_properties.rule = {type:"recurring", id, days_of_week, name}`, `status: "scheduled"`, and `program{title: rule name, start_time, end_time}`. Deleting the rule removes its future recordings.

**Series rule** (`GET/POST/DELETE /api/channels/series-rules/`): `POST {tvg_id?, title, mode, title_mode, description?, description_mode?, untagged_is_new?, channel_id?, epg_source_id?}`; `mode` in `all|new`, `title_mode` in `exact|contains|search|regex`, `description_mode` in `contains|search|regex`. Response `200 {"success":true,"rules":[{tvg_id,mode,title,title_mode,description,description_mode}]}` (the whole list; re-posting an identical rule is a no-op). `GET` returns `{"rules":[…]}`. `DELETE ?tvg_id=&title=[&epg_source_id=]` → `200 {"success":true,"rules":[],"removed":N}` (removes the rule and its future recordings). **Creation does not schedule anything by itself on the local server**: matching recordings appeared only after `POST /api/channels/series-rules/evaluate/ {}` → `{"success":true,"scheduled":5,"details":[{tvg_id,title,status:"ok",created:5}]}`. Evaluation-created recordings have no `status` and no `rule` tag; they carry `custom_properties{season,episode,onscreen_episode,program{id,title,season,episode,tvg_id,sub_title,description,epg_source_id,start_time,end_time,…}}`, so matching them back to a rule is by title + tvg_id. `POST …/preview/` takes the same body plus `limit` and returns `{matches:[{id,tvg_id,title,sub_title,description,start_time,end_time,season,episode,is_new}],total,limit,epg_found,warn}` without persisting.

## App data

| Call | Observed |
|---|---|
| `GET /api/accounts/users/me/app-data/` | `{limits{max_value_bytes:131072, max_total_bytes:1048576, max_apps:25, max_depth:8, key_min_length:8, key_max_length:64}, total_bytes, apps[{key,bytes}]}` [`app_data_list`] |
| `PUT/GET/DELETE /api/accounts/users/me/app-data/{key}/` | body and response are the raw JSON value [`app_data_get`]. **Keys must be 8-64 chars.** |

Not captured: recording comskip/refresh-artwork/update-metadata, VOD image bytes.

## Local test server data

13 channels (10 from the synthetic playlist, 3 from the synthetic Xtream account `XC Catchup`, `XC News`, `XC Sports`) (`tools/fixtures/streams/channels.json`): `XC Catchup` (Xtream account, plays) and `Catchup One` (plain M3U, `is_catchup` true but not playable, see above) have
`is_catchup: true, catchup_days: 3`; `Redirect One` has `stream_profile_id: 4` (Redirect); the rest use the
default profile. The EPG spans 60 h back to 48 h ahead in hourly slots, regenerated relative to "now" each time
the XMLTV is fetched, so fixture timestamps in `tools/fixtures/dispatcharr/` are frozen at capture time
(2026-11-19) and tests must not assume they are current.
