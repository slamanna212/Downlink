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
