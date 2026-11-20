#!/usr/bin/env python3
"""Derive tools/fixtures/xtream/panel-b/ from the base Xtream fixtures: the SAME synthetic lineup and ids, encoded the way a
different (third-party) panel does it. A correct client maps both sets to identical models. Deterministic; outputs are committed."""
import datetime as dt, json, os
HERE = os.path.dirname(os.path.abspath(__file__)); X = os.path.join(HERE, "xtream"); OUT = os.path.join(X, "panel-b")
load = lambda n: json.load(open(os.path.join(X, n + ".json")))
def save(n, v): json.dump(v, open(os.path.join(OUT, n + ".json"), "w", encoding="utf-8"), indent=2, ensure_ascii=False); open(os.path.join(OUT, n + ".json"), "a").write("\n")
dtstr = lambda e: dt.datetime.fromtimestamp(int(e), dt.timezone.utc).strftime("%Y-%m-%d %H:%M:%S")
s = str
def reorder(d, first):  # put the listed keys last, reversing the base key order a bit
    return {k: d[k] for k in reversed(list(d)) if k in d}

# login: ints instead of strings, bool-ish auth, extra server_info fields, richer output formats, different ordering
b = load("player_api_login"); ui, si = b["user_info"], b["server_info"]
save("player_api_login", {"server_info": {"xui": True, "version": "1.5.12", "revision": 2, "url": si["url"], "port": int(si["port"]), "https_port": int(si["https_port"]),
    "server_protocol": si["server_protocol"], "rtmp_port": int(si["rtmp_port"]), "timestamp_now": s(si["timestamp_now"]), "time_now": "2026-01-01 00:00:00",
    "timezone": si["timezone"]},
  "user_info": {"username": ui["username"], "password": ui["password"], "message": "", "auth": True, "status": "Active", "exp_date": int(ui["exp_date"]),
    "is_trial": False, "active_cons": 0, "created_at": int(ui["created_at"]), "max_connections": int(ui["max_connections"]),
    "allowed_output_formats": ["m3u8", "ts", "rtmp"]}})
save("error_auth_failed", {"user_info": {"auth": 0, "status": "Disabled", "message": "Invalid credentials"}})

# categories: category_id as int, parent_id as string
save("get_live_categories", [{"category_name": c["category_name"], "category_id": int(c["category_id"]), "parent_id": s(c["parent_id"])} for c in load("get_live_categories")])

# live: numbers as strings (num, stream_id, tv_archive_duration), added as datetime string, ints for flags, null instead of "" for icon/epg id,
# extra fields, key order reversed, category_ids dropped
live = []
for c in load("get_live_streams"):
    d = {"num": s(c["num"]), "name": c["name"], "stream_type": "live", "stream_id": s(c["stream_id"]), "stream_icon": c.get("stream_icon") or None,
         "epg_channel_id": c.get("epg_channel_id") or None, "added": dtstr(c["added"]) if c.get("added") else None, "is_adult": int(c["is_adult"]),
         "category_id": None if c["category_id"] is None else s(c["category_id"]), "custom_sid": None,
         "tv_archive": int(c["tv_archive"]), "direct_source": "", "tv_archive_duration": int(c["tv_archive_duration"]), "thumbnail": "", "rating": None}
    if "epg_channel_id" not in c: d.pop("epg_channel_id")     # same missing-field quirk as the base set
    live.append(reorder(d, []))
save("get_live_streams", live)

# vod: stream_id string, rating numeric, added epoch int, flags bool, extra fields, backdrop/cover quirks
vod = []
for m in load("get_vod_streams"):
    vod.append(reorder({"num": s(m["num"]), "name": m["name"], "title": m["name"], "year": m["name"].rsplit("(", 1)[-1].rstrip(")"), "stream_type": "movie",
        "stream_id": s(m["stream_id"]), "stream_icon": m["stream_icon"], "rating": float(m["rating"]) if m["rating"] else 0, "rating_5based": s(m["rating_5based"]),
        "added": int(m["added"]), "is_adult": False, "category_id": s(m["category_id"]), "container_extension": m["container_extension"],
        "custom_sid": "", "direct_source": ""}, []))
save("get_vod_streams", vod)

# series: numeric strings, empty backdrop_path as {} instead of [], last_modified as datetime, series_id as string
ser = []
for x in load("get_series"):
    ser.append(reorder({**{k: v for k, v in x.items() if k not in ("series_id", "last_modified", "backdrop_path", "category_ids", "num", "rating_5based")},
        "num": s(x["num"]), "series_id": s(x["series_id"]), "last_modified": dtstr(x["last_modified"]), "rating_5based": s(x["rating_5based"]),
        "backdrop_path": x["backdrop_path"] or {}}, []))
save("get_series", ser)

# series info: seasons as an object keyed by number, episodes list-of-lists for one season / object for several, ids ints, durations ints, info as extra fields
si = load("get_series_info")
seasons = {s_["season_number"] and s(s_["season_number"]): {**s_, "episode_count": int(s_["episode_count"]), "id": s(s_["id"]), "season_number": s(s_["season_number"])} for s_ in si["seasons"]}
eps = {k: [{**e, "id": int(e["id"]), "episode_num": s(e["episode_num"]), "added": dtstr(e["added"]), "season": s(e["season"]),
            "info": {**e["info"], "duration_secs": int(e["info"]["duration_secs"]), "rating": s(e["info"]["rating"]), "bitrate": s(e["info"]["bitrate"]), "crew": "", "audio": {}, "video": {}}}
           for e in v] for k, v in si["episodes"].items()}
save("get_series_info", {"episodes": eps, "info": {**si["info"], "backdrop_path": si["info"]["backdrop_path"], "last_modified": dtstr(si["info"]["last_modified"]), "rating_5based": 3.75, "tmdb": "", "episode_run_time": 45},
                         "seasons": seasons})

# short epg: base64 title/description (same as base) but start/end as epoch INTs only (no datetime strings), extra fields; empty as {} not []
epg = load("get_short_epg")["epg_listings"]
save("get_short_epg", {"epg_listings": [{"id": e["id"], "epg_id": e["epg_id"], "title": e["title"], "lang": "", "start": int(e["start_timestamp"]), "end": int(e["stop_timestamp"]),
    "description": e["description"], "channel_id": e["channel_id"], "start_timestamp": int(e["start_timestamp"]), "stop_timestamp": int(e["stop_timestamp"]),
    "now_playing": 1 if i == 0 else 0, "has_archive": 0} for i, e in enumerate(epg)]})
save("get_short_epg_empty", {"epg_listings": {}})
print("ok")
