#!/usr/bin/env python3
"""Capture scrubbed JSON responses from the local test Dispatcharr into tools/fixtures/dispatcharr/.

Synthetic data only (the container only holds synthetic content), but hosts, IPs, keys, tokens and
container ids are still replaced with placeholders. Never requests a stream endpoint (/proxy/ts/stream,
/proxy/hls, /proxy/vod, /proxy/catchup/<uuid>). Catch-up session create/position/delete is an API call, not a stream.
DVR capture schedules a ~90 s one-off recording on Test Kids (Dispatcharr pulls the synthetic channel itself) and stops it.

  capture-fixtures.py                    capture everything
  capture-fixtures.py --scrub-stdin NAME scrub JSON on stdin into NAME.json (for the manual ts/status capture)
"""
import json, os, re, sys, datetime as dt, urllib.request, urllib.error

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "fixtures", "dispatcharr")

def env():
    d = {}
    for line in open(os.path.join(HERE, ".env.local")):
        if "=" in line and not line.startswith("#"):
            k, v = line.strip().split("=", 1); d[k] = v
    return d
E = env() if os.path.exists(os.path.join(HERE, ".env.local")) else {}

def scrub_text(t):
    subs = [
        (re.escape(E.get("DISPATCHARR_API_KEY", "\0")), "REDACTED-API-KEY"),
        (re.escape(E.get("DISPATCHARR_PASSWORD", "\0")), "REDACTED-PASSWORD"),
        (r"http://localhost:9191", "https://dispatcharr.test"),
        (r"host\.docker\.internal:\d+", "streams.dispatcharr.test"),
        (r"172\.17\.0\.1(:\d+)?", "192.0.2.1"),
        (r"\blocalhost:\d+", "dispatcharr.test"),
        (r"eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+", "REDACTED.JWT.TOKEN"),
        (r'"owner": "[0-9a-f]{12}:\d+"', '"owner": "dispatcharr-test:1"'),
        (r'"ip_address": "[0-9.]+"', '"ip_address": "192.0.2.10"'),
        (r"curl/[0-9.]+", "downlink-fixtures/0"),
        (r"downlink-admin", "test-admin"),
        # provider credentials embedded in VOD/episode m3u_account objects and stream URLs
        (r'"password": "[^"]*"', '"password": "REDACTED-PASSWORD"'),
        (r'"username": "[^"]*"', '"username": "REDACTED-USER"'),
        (r"testuser", "REDACTED-USER"), (r"testpass", "REDACTED-PASSWORD"),
        (r"/data/recordings", "/data/recordings"),
    ]
    for pat, rep in subs: t = re.sub(pat, rep, t)
    return t

def save(name, body):
    os.makedirs(OUT, exist_ok=True)
    t = json.dumps(body, indent=2, sort_keys=False) if not isinstance(body, str) else body
    with open(os.path.join(OUT, name + ".json"), "w") as f: f.write(scrub_text(t) + "\n")
    print("saved", name)

def call(method, path, body=None, auth=True, ok=(200, 201, 204)):
    h = {"Content-Type": "application/json"}
    if auth: h["X-API-Key"] = E["DISPATCHARR_API_KEY"]
    req = urllib.request.Request(E["DISPATCHARR_URL"] + path, method=method, headers=h,
                                 data=None if body is None else json.dumps(body).encode())
    try:
        with urllib.request.urlopen(req, timeout=60) as r: code, txt = r.status, r.read().decode()
    except urllib.error.HTTPError as e: code, txt = e.code, e.read().decode()
    try: js = json.loads(txt) if txt else None
    except ValueError: js = txt
    if code not in ok: print(f"  ! {method} {path} -> {code} {str(js)[:150]}")
    return code, js

def iso(t): return t.strftime("%Y-%m-%dT%H:%M:%SZ")

def capture_vod():
    """Needs VOD ingested (bootstrap does it). provider-info calls are what make Dispatcharr fetch details/episodes."""
    save("vod_movies", call("GET", "/api/vod/movies/?page=1&page_size=50")[1])
    save("vod_series", call("GET", "/api/vod/series/?page=1&page_size=50")[1])
    save("vod_categories", call("GET", "/api/vod/categories/")[1])
    save("vod_all", call("GET", "/api/vod/all/?page=1&page_size=50")[1])
    save("vod_vodlogos", call("GET", "/api/vod/vodlogos/?page=1&page_size=50")[1])
    movies = call("GET", "/api/vod/movies/?page=1&page_size=50")[1]["results"]
    series = call("GET", "/api/vod/series/?page=1&page_size=50")[1]["results"]
    if movies:
        m = movies[0]["id"]
        save("vod_movie_detail", call("GET", f"/api/vod/movies/{m}/")[1])
        save("vod_movie_providers", call("GET", f"/api/vod/movies/{m}/providers/")[1])
        save("vod_movie_provider_info", call("GET", f"/api/vod/movies/{m}/provider-info/")[1])
    if series:
        sid = series[0]["id"]
        save("vod_series_detail", call("GET", f"/api/vod/series/{sid}/")[1])
        save("vod_series_provider_info", call("GET", f"/api/vod/series/{sid}/provider-info/")[1])
        save("vod_series_episodes", call("GET", f"/api/vod/series/{sid}/episodes/")[1])
        save("vod_episodes", call("GET", "/api/vod/episodes/?page=1&page_size=3")[1])

def capture_dvr(byname):
    import time
    iso = lambda t: t.strftime("%Y-%m-%dT%H:%M:%SZ")
    now = dt.datetime.now(dt.timezone.utc)
    ok2 = (200, 201, 400)
    recs = call("GET", "/api/channels/recordings/")[1]
    save("recordings_list", recs)
    kind = lambda r: (r["custom_properties"].get("rule") or {}).get("type")
    first = lambda f: next((r for r in recs if f(r)), None)
    for name, rec in (("recording_scheduled_recurring", first(lambda r: kind(r) == "recurring")),
                      ("recording_scheduled_series", first(lambda r: (r["custom_properties"].get("program") or {}).get("title") == "Pattern Playhouse")),
                      ("recording_completed", first(lambda r: r["custom_properties"].get("status") == "completed"))):
        if rec: save(name, rec)
    code, up = call("POST", "/api/channels/recordings/", {"channel": byname["Test Kids"]["id"], "start_time": iso(now + dt.timedelta(hours=3)), "end_time": iso(now + dt.timedelta(hours=4)),
                                                          "custom_properties": {"program": {"title": "Synthetic Future Recording", "description": "one-off"}}})
    save("recording_scheduled_oneoff", up)
    call("DELETE", f"/api/channels/recordings/{up['id']}/")
    done = first(lambda r: r["custom_properties"].get("status") == "completed")
    if done:
        save("recording_extend_finished_error", call("POST", f"/api/channels/recordings/{done['id']}/extend/", {"extra_minutes": 1}, ok=ok2)[1])
    # live one-off: starts now, 90 s; capture the in-progress shapes, extend it, stop it
    ch = byname["Test Kids"]
    code, rec = call("POST", "/api/channels/recordings/", {"channel": ch["id"], "start_time": iso(now - dt.timedelta(seconds=2)), "end_time": iso(now + dt.timedelta(seconds=90)),
                                                           "custom_properties": {"program": {"title": "Synthetic In Progress Recording", "description": "capture"}}})
    save("recording_create_now", rec)
    rid = rec["id"]
    for _ in range(30):
        cur = call("GET", f"/api/channels/recordings/{rid}/")[1]
        if cur["custom_properties"].get("status") == "recording" and cur["custom_properties"].get("file_url"): break
        time.sleep(2)
    time.sleep(8)  # let a few HLS segments land
    save("recording_in_progress", call("GET", f"/api/channels/recordings/{rid}/")[1])
    req = urllib.request.Request(E["DISPATCHARR_URL"] + f"/api/channels/recordings/{rid}/hls/index.m3u8", headers={"X-API-Key": E["DISPATCHARR_API_KEY"]})
    with urllib.request.urlopen(req) as r: save("recording_in_progress_hls_playlist", json.dumps({"content_type": r.headers["Content-Type"], "body": r.read().decode()}, indent=2))
    save("recording_extend_in_progress_wrong_field", call("POST", f"/api/channels/recordings/{rid}/extend/", {"minutes": 1}, ok=ok2)[1])
    save("recording_extend_in_progress", call("POST", f"/api/channels/recordings/{rid}/extend/", {"extra_minutes": 1}, ok=ok2)[1])
    save("recording_stop", call("POST", f"/api/channels/recordings/{rid}/stop/")[1])
    time.sleep(6)
    save("recording_after_stop", call("GET", f"/api/channels/recordings/{rid}/")[1])
    # rules
    save("recurring_rules", call("GET", "/api/channels/recurring-rules/")[1])
    save("series_rules", call("GET", "/api/channels/series-rules/")[1])
    save("series_rule_create", call("POST", "/api/channels/series-rules/", {"tvg_id": "test-news.test", "title": "Pattern Playhouse", "mode": "all", "title_mode": "exact"})[1])
    save("series_rule_preview", call("POST", "/api/channels/series-rules/preview/", {"tvg_id": "test-news.test", "title": "Pattern Playhouse", "mode": "all", "title_mode": "exact", "limit": 3}, ok=(200, 201))[1])
    save("recurring_rule_missing_dates_error", call("POST", "/api/channels/recurring-rules/", {"channel": ch["id"], "days_of_week": [0], "start_time": "20:00:00", "end_time": "21:00:00", "name": "x"}, ok=ok2)[1])
    save("recording_past_error", call("POST", "/api/channels/recordings/", {"channel": ch["id"], "start_time": iso(now - dt.timedelta(minutes=10)), "end_time": iso(now - dt.timedelta(minutes=5))}, ok=ok2)[1])

def main():
    if len(sys.argv) == 3 and sys.argv[1] == "--scrub-stdin":
        save(sys.argv[2], json.dumps(json.load(sys.stdin), indent=2)); return
    now = dt.datetime.now(dt.timezone.utc)
    # identity / connect sequence
    save("core_version", call("GET", "/api/core/version/", auth=False)[1])
    save("users_me", call("GET", "/api/accounts/users/me/")[1])
    save("core_timezones", {"note": "truncated", "sample": (call("GET", "/api/core/timezones/")[1] or [])[:5]}
         if False else call("GET", "/api/core/timezones/")[1])
    st = call("GET", "/api/core/settings/")[1]
    save("core_settings", [s for s in st if s["key"] in ("stream_settings", "system_settings")])
    save("core_streamprofiles", call("GET", "/api/core/streamprofiles/")[1])
    # token endpoints: shape only, values placeholders
    save("accounts_token", {"access": "REDACTED.JWT.TOKEN", "refresh": "REDACTED.JWT.TOKEN"})
    save("accounts_token_refresh", {"access": "REDACTED.JWT.TOKEN"})
    # channels
    c, chans = call("GET", "/api/channels/channels/?page=1&page_size=500")
    save("channels_page", chans)
    save("channels_search_unpaginated", call("GET", "/api/channels/channels/?search=news")[1])
    save("channels_by_group", call("GET", "/api/channels/channels/?page=1&page_size=500&channel_group=Test%20Lab")[1])
    byname = {ch["name"]: ch for ch in chans["results"]}
    save("channel_streams", call("GET", f"/api/channels/channels/{byname['Channel One']['id']}/streams/")[1])
    save("channel_streams_catchup_m3u", call("GET", f"/api/channels/channels/{byname['Catchup One']['id']}/streams/")[1])
    save("channel_streams_catchup", call("GET", f"/api/channels/channels/{byname['XC Catchup']['id']}/streams/")[1])
    save("channel_redirect", byname["Redirect One"])
    save("channels_groups", call("GET", "/api/channels/groups/")[1])
    save("channels_profiles", call("GET", "/api/channels/profiles/")[1])
    save("channels_logos", call("GET", "/api/channels/logos/?page=1&page_size=50")[1])
    # guide
    save("epg_grid_default", call("GET", "/api/epg/grid/")[1])
    save("epg_grid_window", call("GET", f"/api/epg/grid/?start={iso(now - dt.timedelta(hours=26))}&end={iso(now + dt.timedelta(hours=6))}")[1])
    save("epg_grid_prev_days", call("GET", "/api/epg/grid/?prev_days=2&days=1")[1])
    uuids = [ch["uuid"] for ch in chans["results"][:3]]
    save("epg_current_programs_uuids", call("POST", "/api/epg/current-programs/", {"channel_uuids": uuids}, ok=(200, 201))[1])
    save("epg_current_programs_all", call("POST", "/api/epg/current-programs/", {}, ok=(200, 201))[1])
    save("epg_programs_search_title", call("GET", "/api/epg/programs/search/?title=colour&page_size=5")[1])
    save("epg_programs_search_airing", call("GET", f"/api/epg/programs/search/?airing_at={iso(now)}&page_size=5")[1])
    save("epg_programs_search_window", call("GET", f"/api/epg/programs/search/?start_after={iso(now)}&start_before={iso(now + dt.timedelta(hours=3))}&channel=news&page_size=5")[1])
    # catch-up (API only: no stream request)
    cu = byname["XC Catchup"]["uuid"]
    start = (now - dt.timedelta(hours=26)).replace(minute=0, second=0, microsecond=0)
    code, sess = call("POST", "/api/catchup/sessions/", {"channel_uuid": cu, "start": iso(start), "duration": 60})
    save("catchup_session_create", sess)
    if code == 201:
        save("catchup_session_position_no_playback", call("POST", f"/api/catchup/sessions/{sess['session_id']}/position/", {"position_secs": 120}, ok=(200, 201, 204))[1] or {})
        call("DELETE", f"/api/catchup/sessions/{sess['session_id']}/")
    save("catchup_session_unsupported_channel", call("POST", "/api/catchup/sessions/", {"channel_uuid": byname["Channel One"]["uuid"], "start": iso(start)}, ok=(400,))[1])
    capture_vod()
    capture_dvr(byname)
    # app data
    call("PUT", "/api/accounts/users/me/app-data/downlink-fixture/", {"hello": "world"}, ok=(200, 201, 204))
    save("app_data_get", call("GET", "/api/accounts/users/me/app-data/downlink-fixture/")[1])
    save("app_data_list", call("GET", "/api/accounts/users/me/app-data/")[1])
    call("DELETE", "/api/accounts/users/me/app-data/downlink-fixture/")
    save("proxy_ts_status_idle", call("GET", "/proxy/ts/status")[1])
    save("proxy_hls_sessions", {"note": "DELETE /api/proxy/hls/sessions/{token}/ returns 204; no list endpoint"})

main()
