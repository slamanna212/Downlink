#!/usr/bin/env python3
"""Capture scrubbed JSON responses from the local test Dispatcharr into tools/fixtures/dispatcharr/.

Synthetic data only (the container only holds synthetic content), but hosts, IPs, keys, tokens and
container ids are still replaced with placeholders. Never requests a stream endpoint (/proxy/ts/stream,
/proxy/hls, /proxy/vod, /proxy/catchup/<uuid>). Catch-up session create/position/delete is an API call, not a stream.

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
    save("channel_streams_catchup", call("GET", f"/api/channels/channels/{byname['Catchup One']['id']}/streams/")[1])
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
    cu = byname["Catchup One"]["uuid"]
    start = (now - dt.timedelta(hours=26)).replace(minute=0, second=0, microsecond=0)
    code, sess = call("POST", "/api/catchup/sessions/", {"channel_uuid": cu, "start": iso(start), "duration": 60})
    save("catchup_session_create", sess)
    if code == 201:
        save("catchup_session_position_no_playback", call("POST", f"/api/catchup/sessions/{sess['session_id']}/position/", {"position_secs": 120}, ok=(200, 201, 204))[1] or {})
        call("DELETE", f"/api/catchup/sessions/{sess['session_id']}/")
    save("catchup_session_unsupported_channel", call("POST", "/api/catchup/sessions/", {"channel_uuid": byname["Channel One"]["uuid"], "start": iso(start)}, ok=(400,))[1])
    # VOD (no VOD content on the synthetic server: empty paginated envelopes)
    for n in ("movies", "series", "categories"):
        save(f"vod_{n}", call("GET", f"/api/vod/{n}/?page=1&page_size=50")[1])
    # DVR: schedule a future recording + rules, capture, remove
    ch = byname["Channel One"]
    s, e = now + dt.timedelta(hours=4), now + dt.timedelta(hours=5)
    code, rec = call("POST", "/api/channels/recordings/", {"channel": ch["id"], "start_time": iso(s), "end_time": iso(e), "custom_properties": {}}, ok=(200, 201))
    save("recordings_create", rec)
    save("recordings_list", call("GET", "/api/channels/recordings/")[1])
    if code in (200, 201) and isinstance(rec, dict) and rec.get("id"):
        call("DELETE", f"/api/channels/recordings/{rec['id']}/")
    save("recurring_rules", call("GET", "/api/channels/recurring-rules/")[1])
    save("series_rules", call("GET", "/api/channels/series-rules/")[1])
    # app data
    call("PUT", "/api/accounts/users/me/app-data/downlink-fixture/", {"hello": "world"}, ok=(200, 201, 204))
    save("app_data_get", call("GET", "/api/accounts/users/me/app-data/downlink-fixture/")[1])
    save("app_data_list", call("GET", "/api/accounts/users/me/app-data/")[1])
    call("DELETE", "/api/accounts/users/me/app-data/downlink-fixture/")
    save("proxy_ts_status_idle", call("GET", "/proxy/ts/status")[1])
    save("proxy_hls_sessions", {"note": "DELETE /api/proxy/hls/sessions/{token}/ returns 204; no list endpoint"})

main()
