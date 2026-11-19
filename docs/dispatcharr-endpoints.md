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
| 5 | Catch-up from M3U tags | Dispatcharr's M3U ingest reads only `tv_archive="1"` and `tv_archive_duration="N"` for `is_catchup`/`catchup_days` (XC accounts: `tv_archive`, `tv_archive_duration` from `get_live_streams`). `catchup="default"` / `catchup-days` / `catchup-source` attributes on a plain M3U are **ignored**. The channel's `is_catchup` is rolled up from its active streams. |
| 6 | `POST …/{id}/position/` `{…}` | Body is `{"position_secs": <number>}`. Returns 404 `{"error":"No active playback for this session"}` until the player has made its first GET on `playback_url`; it cannot be called right after create. |
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

## VOD, DVR, app data

| Call | Observed |
|---|---|
| `GET /api/vod/movies/`, `/series/` | paginated, empty locally (no VOD provider in the synthetic server) [`vod_movies`, `vod_series`]. Item shapes: see the OpenAPI schema. |
| `GET /api/vod/categories/` | bare array `{id,name,category_type ("movie"\|"series"),category_type_display,m3u_accounts[]}`; a default "Uncategorized" per type [`vod_categories`] |
| `GET /api/vod/series/{id}/episodes/`, `/api/vod/{movies,series,episodes}/{id}/image` | not exercised (no data) |
| `POST /api/channels/recordings/` | `{channel:<id>, start_time, end_time, custom_properties:{}}` → `{id,start_time,end_time,task_id,custom_properties,channel}` [`recordings_create`]; `GET` → bare array [`recordings_list`] |
| `GET /api/channels/recurring-rules/` | bare array [`recurring_rules`] |
| `GET /api/channels/series-rules/` | `{"rules":[…]}` [`series_rules`] |
| `GET /api/accounts/users/me/app-data/` | `{limits{max_value_bytes:131072, max_total_bytes:1048576, max_apps:25, max_depth:8, key_min_length:8, key_max_length:64}, total_bytes, apps[{key,bytes}]}` [`app_data_list`] |
| `PUT/GET/DELETE /api/accounts/users/me/app-data/{key}/` | body and response are the raw JSON value [`app_data_get`]. **Keys must be 8-64 chars.** |

Not captured (would need real content): recording stop/extend/file/HLS, recurring/series rule creation,
VOD item bodies, logo image bytes.

## Local test server data

10 channels from the synthetic playlist (`tools/fixtures/streams/channels.json`): `Catchup One` has
`is_catchup: true, catchup_days: 3`; `Redirect One` has `stream_profile_id: 4` (Redirect); the rest use the
default profile. The EPG spans 60 h back to 48 h ahead in hourly slots, regenerated relative to "now" each time
the XMLTV is fetched, so fixture timestamps in `tools/fixtures/dispatcharr/` are frozen at capture time
(2026-11-19) and tests must not assume they are current.
