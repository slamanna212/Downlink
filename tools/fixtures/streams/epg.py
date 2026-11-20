"""Synthetic XMLTV/M3U generation relative to the current time. Synthetic names only."""
import json, os, datetime as dt
from xml.sax.saxutils import escape

HERE = os.path.dirname(os.path.abspath(__file__))
CHANNELS = json.load(open(os.path.join(HERE, "channels.json")))

TITLES = ["Morning Test Show", "The Synthetic Hour", "Pattern Playhouse", "Colour Bars Live",
          "Sine Wave Cinema", "Late Night Noise", "Test Pattern Theatre", "Bitrate Briefing"]

def _fmt(t): return t.strftime("%Y%m%d%H%M%S +0000")

def xmltv(now=None, back_hours=60, fwd_hours=48, slot_min=60):
    """Programmes on slot_min boundaries covering [now-back_hours, now+fwd_hours] so past, now and future exist."""
    now = now or dt.datetime.now(dt.timezone.utc)
    base = now.replace(minute=0, second=0, microsecond=0)
    start = base - dt.timedelta(hours=back_hours)
    out = ['<?xml version="1.0" encoding="UTF-8"?>', '<tv generator-info-name="downlink-fixtures">']
    for c in CHANNELS:
        out.append(f'<channel id="{c["tvg_id"]}"><display-name>{escape(c["name"])}</display-name></channel>')
    for ci, c in enumerate(CHANNELS):
        t = start
        i = 0
        while t < base + dt.timedelta(hours=fwd_hours):
            e = t + dt.timedelta(minutes=slot_min)
            title = TITLES[(i + ci) % len(TITLES)]
            out.append(
                f'<programme start="{_fmt(t)}" stop="{_fmt(e)}" channel="{c["tvg_id"]}">'
                f'<title lang="en">{escape(title)}</title>'
                f'<sub-title>Episode {i % 20 + 1}</sub-title>'
                f'<desc lang="en">Synthetic programme {i} on {escape(c["name"])}.</desc>'
                f'<category lang="en">{escape(c["group"])}</category>'
                f'<episode-num system="onscreen">S01E{i % 20 + 1:02d}</episode-num>'
                + ('<new/>' if i % 5 == 0 else '') + '</programme>')
            t, i = e, i + 1
    out.append('</tv>')
    return "\n".join(out)

def m3u(stream_host, epg_url=None, redirect_host=None):
    """stream_host: host:port placed in stream URLs. Dispatcharr-style tv_archive attrs for catch-up channels."""
    hdr = '#EXTM3U' + (f' x-tvg-url="{epg_url}"' if epg_url else '')
    out = [hdr]
    for c in CHANNELS:
        if c.get("xc_id"): continue
        host = redirect_host if (c.get("redirect") and redirect_host) else stream_host
        attrs = (f'tvg-id="{c["tvg_id"]}" tvg-name="{c["name"]}" tvg-chno="{c["number"]}" '
                 f'group-title="{c["group"]}"')
        if c.get("catchup_days"):
            attrs += f' tv_archive="1" tv_archive_duration="{c["catchup_days"]}"'
        out.append(f'#EXTINF:-1 {attrs},{c["name"]}')
        out.append(f'http://{host}/ch/{c["slug"]}.ts')
    return "\n".join(out) + "\n"


XC_USER, XC_PASS = "testuser", "testpass"

def xc_channels(): return [c for c in CHANNELS if c.get("xc_id")]

def xc_categories():
    return [{"category_id": "1", "category_name": "Test Xtream", "parent_id": 0}]

def xc_live_streams(host, now=None):
    now = int((now or dt.datetime.now(dt.timezone.utc)).timestamp())
    out = []
    for i, c in enumerate(xc_channels(), 1):
        out.append({"num": i, "name": c["name"], "stream_type": "live", "stream_id": c["xc_id"], "stream_icon": "",
                    "epg_channel_id": c["tvg_id"], "added": str(now - 86400), "is_adult": "0", "category_id": "1",
                    "category_ids": [1], "custom_sid": "", "tv_archive": 1 if c.get("catchup_days") else 0,
                    "direct_source": "", "tv_archive_duration": c.get("catchup_days", 0)})
    return out

def xc_login(host):
    now = dt.datetime.now(dt.timezone.utc)
    h, _, p = host.partition(":")
    return {"user_info": {"username": XC_USER, "password": XC_PASS, "message": "", "auth": 1, "status": "Active",
                          "exp_date": str(int(now.timestamp()) + 365 * 86400), "is_trial": "0", "active_cons": "0",
                          "created_at": str(int(now.timestamp()) - 86400), "max_connections": "5",
                          "allowed_output_formats": ["ts"]},
            "server_info": {"url": h, "port": p or "80", "https_port": "443", "server_protocol": "http", "rtmp_port": "1935",
                            "timezone": "UTC", "timestamp_now": int(now.timestamp()),
                            "time_now": now.strftime("%Y-%m-%d %H:%M:%S")}}


# ---------------- Xtream VOD (synthetic) ----------------
VOD_CATEGORIES = [{"category_id": "10", "category_name": "Test Movies", "parent_id": 0},
                  {"category_id": "11", "category_name": "Test Documentaries", "parent_id": 0}]
SERIES_CATEGORIES = [{"category_id": "20", "category_name": "Test Series", "parent_id": 0}]
MOVIES = [  # id, name, cat, ext, year, poster
    (2001, "Test Movie One", "10", "mp4", 2021, "movie1"), (2002, "Test Movie Two", "10", "mkv", 2022, "movie2"),
    (2003, "Test Documentary Three", "11", "mp4", 2019, "movie3"), (2004, "Test Movie Four", "10", "mkv", 2023, "movie4")]
SERIES = [(3001, "Test Series Alpha", {1: 3, 2: 2}, "series1", 2020), (3002, "Test Series Beta", {1: 2}, "series2", 2024)]
_T0 = 1767225600

def _episode_id(sid, season, ep): return sid * 100 + season * 10 + ep   # 300111 ...

def xc_vod_streams(host):
    return [{"num": i, "name": f"{n} ({y})", "stream_type": "movie", "stream_id": sid, "stream_icon": f"http://{host}/img/{p}.png",
             "rating": f"{6 + i % 3}.{i}", "rating_5based": 3.5, "added": str(_T0 + i), "is_adult": "0", "category_id": cat,
             "category_ids": [int(cat)], "container_extension": ext, "custom_sid": None, "direct_source": ""}
            for i, (sid, n, cat, ext, y, p) in enumerate(MOVIES, 1)]

def xc_vod_info(host, vod_id):
    for sid, n, cat, ext, y, p in MOVIES:
        if sid == vod_id:
            return {"info": {"movie_image": f"http://{host}/img/{p}.png", "tmdb_id": "", "name": n, "o_name": n, "releasedate": f"{y}-03-01",
                             "youtube_trailer": "", "director": "Test Director", "actors": "Test Actor", "cast": "Test Actor", "description": f"Synthetic plot for {n}.",
                             "plot": f"Synthetic plot for {n}.", "age": "", "country": "Testland", "genre": "Test", "backdrop_path": [f"http://{host}/img/backdrop.png"],
                             "duration_secs": 20, "duration": "00:00:20", "rating": "7.0", "bitrate": 2000},
                    "movie_data": {"stream_id": sid, "name": n, "added": str(_T0), "category_id": cat, "container_extension": ext, "custom_sid": "", "direct_source": ""}}
    return {}

def xc_series(host):
    return [{"num": i, "name": n, "series_id": sid, "cover": f"http://{host}/img/{p}.png", "plot": f"Synthetic plot for {n}.", "cast": "Test Actor",
             "director": "Test Director", "genre": "Test", "releaseDate": f"{y}-01-01", "release_date": f"{y}-01-01", "last_modified": str(_T0),
             "rating": "7.5", "rating_5based": 3.75, "backdrop_path": [f"http://{host}/img/backdrop.png"], "youtube_trailer": "",
             "episode_run_time": "1", "category_id": "20", "category_ids": [20]} for i, (sid, n, seasons, p, y) in enumerate(SERIES, 1)]

def xc_series_info(host, series_id):
    for sid, n, seasons, p, y in SERIES:
        if sid == series_id:
            return {"seasons": [{"air_date": f"{y}-01-01", "episode_count": str(c), "id": s, "name": f"Season {s}", "overview": "", "season_number": s,
                                 "cover": f"http://{host}/img/{p}.png"} for s, c in seasons.items()],
                    "info": {"name": n, "cover": f"http://{host}/img/{p}.png", "plot": f"Synthetic plot for {n}.", "cast": "Test Actor", "director": "Test Director",
                             "genre": "Test", "releaseDate": f"{y}-01-01", "last_modified": str(_T0), "rating": "7.5", "rating_5based": "3.75",
                             "backdrop_path": [f"http://{host}/img/backdrop.png"], "youtube_trailer": "", "episode_run_time": "1", "category_id": "20"},
                    "episodes": {str(s): [{"id": str(_episode_id(sid, s, e)), "episode_num": e, "title": f"{n} - S{s:02d}E{e:02d}", "container_extension": "mp4",
                                           "info": {"movie_image": f"http://{host}/img/{p}.png", "plot": f"Synthetic episode {e}.", "duration_secs": 20,
                                                    "duration": "00:00:20", "releasedate": f"{y}-01-0{e}", "rating": 7.0, "bitrate": 2000},
                                           "custom_sid": "", "added": str(_T0), "season": s, "direct_source": ""} for e in range(1, c + 1)] for s, c in seasons.items()}}
    return {}
