#!/usr/bin/env python3
"""Generate the committed synthetic Xtream / M3U / XMLTV edge-case fixtures (deterministic).
All names, hosts (.test / .invalid) and credentials are synthetic. Re-run to regenerate; outputs are committed.
The 20k-entry M3U is NOT committed: run gen-large-m3u.py.
"""
import base64, gzip, json, os
HERE = os.path.dirname(os.path.abspath(__file__))
def w(rel, content, mode="w", enc="utf-8"):
    p = os.path.join(HERE, rel); os.makedirs(os.path.dirname(p), exist_ok=True)
    if isinstance(content, (dict, list)): content = json.dumps(content, indent=2, ensure_ascii=False) + "\n"
    if "b" in mode: open(p, mode).write(content)
    else: open(p, mode, encoding=enc, newline="").write(content)

# ---------------- Xtream ----------------
T0 = 1767225600  # 2026-01-01T00:00:00Z
cats = [{"category_id": str(i), "category_name": n, "parent_id": 0} for i, n in
        [(1, "Test Entertainment"), (2, "Test News"), (3, "Test Sports"), (4, "Adult Test"), (5, "Radio Test"), (6, "Test Lab")]]
w("xtream/player_api_login.json", {
  "user_info": {"username": "testuser", "password": "testpass", "message": "Welcome", "auth": 1, "status": "Active",
                "exp_date": str(T0 + 86400 * 365), "is_trial": "0", "active_cons": "0", "created_at": str(T0 - 86400 * 30),
                "max_connections": "2", "allowed_output_formats": ["m3u8", "ts"]},
  "server_info": {"url": "xtream.test", "port": "8080", "https_port": "443", "server_protocol": "http", "rtmp_port": "1935",
                  "timezone": "Europe/London", "timestamp_now": T0, "time_now": "2026-01-01 00:00:00", "process": True}})
w("xtream/get_live_categories.json", cats)
live = []
for i in range(1, 13):
    live.append({"num": i, "name": f"Test Channel {i}", "stream_type": "live", "stream_id": 1000 + i,
                 "stream_icon": f"http://xtream.test/logos/{i}.png" if i % 3 else "", "epg_channel_id": f"test-channel-{i}.test" if i % 4 else "",
                 "added": str(T0 - 86400 * i), "is_adult": "1" if i == 11 else "0", "category_id": str((i % 3) + 1) if i != 11 else "4",
                 "category_ids": [(i % 3) + 1], "custom_sid": "", "tv_archive": 1 if i % 2 == 0 else 0,
                 "direct_source": "", "tv_archive_duration": 3 if i % 2 == 0 else 0})
live[2]["category_id"] = None            # quirk: null category
live[3]["tv_archive"] = "1"              # quirk: string flag
live[3]["tv_archive_duration"] = "7"     # quirk: string duration
live[4]["name"] = "Test Radio FM"; live[4]["category_id"] = "5"
live[5]["added"] = None                  # quirk: null added
del live[6]["epg_channel_id"]            # quirk: missing field
w("xtream/get_live_streams.json", live)
vod = [{"num": i, "name": f"Test Movie {i} (202{i % 6})", "stream_type": "movie", "stream_id": 2000 + i, "stream_icon": f"http://xtream.test/vod/{i}.jpg",
        "rating": f"{5 + i % 5}.{i % 10}", "rating_5based": 3.5, "added": str(T0 - 3600 * i), "is_adult": "0", "category_id": "1", "category_ids": [1],
        "container_extension": "mkv" if i % 2 else "mp4", "custom_sid": None, "direct_source": ""} for i in range(1, 9)]
vod[0]["rating"] = ""; vod[1]["rating_5based"] = "0"; vod[2]["stream_icon"] = None
w("xtream/get_vod_streams.json", vod)
series = [{"num": i, "name": f"Test Series {i}", "series_id": 3000 + i, "cover": f"http://xtream.test/series/{i}.jpg", "plot": f"Synthetic plot {i}.",
           "cast": "Test Actor, Another Actor", "director": "Test Director", "genre": "Drama, Test", "releaseDate": f"202{i % 6}-01-01",
           "release_date": f"202{i % 6}-01-01", "last_modified": str(T0), "rating": "7.5", "rating_5based": 3.75, "backdrop_path": [f"http://xtream.test/bd/{i}.jpg"],
           "youtube_trailer": "", "episode_run_time": "45", "category_id": "1", "category_ids": [1]} for i in range(1, 5)]
series[1]["backdrop_path"] = []
w("xtream/get_series.json", series)
eps = {str(s): [{"id": str(50000 + s * 100 + e), "episode_num": e, "title": f"Test Series 1 - S{s:02d}E{e:02d} - Episode {e}", "container_extension": "mkv",
                 "info": {"movie_image": f"http://xtream.test/ep/{s}{e}.jpg", "plot": "Synthetic episode.", "duration_secs": 2700, "duration": "00:45:00",
                          "releasedate": "2026-01-01", "rating": 7.1, "bitrate": 4000},
                 "custom_sid": "", "added": str(T0), "season": s, "direct_source": ""} for e in range(1, 4)] for s in (1, 2)}
w("xtream/get_series_info.json", {"seasons": [{"air_date": "2026-01-01", "episode_count": "3", "id": 1, "name": "Season 1", "overview": "", "season_number": 1, "cover": "http://xtream.test/s1.jpg"},
                                              {"air_date": "2026-06-01", "episode_count": "3", "id": 2, "name": "Season 2", "overview": "", "season_number": 2, "cover": ""}],
    "info": {"name": "Test Series 1", "cover": "http://xtream.test/series/1.jpg", "plot": "Synthetic plot 1.", "cast": "Test Actor", "director": "Test Director",
             "genre": "Drama, Test", "releaseDate": "2026-01-01", "last_modified": str(T0), "rating": "7.5", "rating_5based": "3.75", "backdrop_path": ["http://xtream.test/bd/1.jpg"],
             "youtube_trailer": "", "episode_run_time": "45", "category_id": "1"}, "episodes": eps})
b64 = lambda s: base64.b64encode(s.encode()).decode()
def listing(i, start, mins, title):
    return {"id": str(9000 + i), "epg_id": "1", "title": b64(title), "lang": "en", "start": time_s(start), "end": time_s(start + mins * 60),
            "description": b64(f"Synthetic description for {title}."), "channel_id": "test-channel-1.test", "start_timestamp": str(start), "stop_timestamp": str(start + mins * 60)}
def time_s(t):
    import datetime as d; return d.datetime.fromtimestamp(t, d.timezone.utc).strftime("%Y-%m-%d %H:%M:%S")
w("xtream/get_short_epg.json", {"epg_listings": [listing(0, T0, 60, "Morning Test Show"), listing(1, T0 + 3600, 60, "The Synthetic Hour"),
                                                  listing(2, T0 + 7200, 30, "Pattern Playhouse"), listing(3, T0 + 9000, 30, "Colour Bars Live")]})
w("xtream/get_short_epg_empty.json", {"epg_listings": []})
w("xtream/error_auth_failed.json", {"user_info": {"auth": 0}})

# ---------------- M3U ----------------
H = "http://iptv.test"
w("m3u/basic.m3u", f'''#EXTM3U x-tvg-url="{H}/epg.xml"
#EXTINF:-1 tvg-id="one.test" tvg-name="One" tvg-logo="{H}/logo/1.png" tvg-chno="1" group-title="Test Entertainment",Channel One
{H}/live/1.ts
#EXTINF:-1 tvg-id="two.test" tvg-name="Two" tvg-chno="2" group-title="Test News",Test News
{H}/live/2.ts
#EXTINF:-1 tvg-id="" group-title="",No Group, No Id
{H}/live/3.ts
''')
w("m3u/aux_lines.m3u", f'''#EXTM3U
#EXTINF:-1 tvg-id="ua.test" group-title="Test Lab",Custom UA Channel
#EXTVLCOPT:http-user-agent=SyntheticAgent/1.0
#EXTVLCOPT:http-referrer={H}/ref
#EXTVLCOPT:http-origin={H}
#EXTVLCOPT:network-caching=1000
{H}/ua/1.m3u8
#EXTINF:-1 tvg-id="drm.test" group-title="Test Lab",DRM Channel
#KODIPROP:inputstream=inputstream.adaptive
#KODIPROP:inputstream.adaptive.manifest_type=mpd
#KODIPROP:inputstream.adaptive.license_type=com.widevine.alpha
#KODIPROP:inputstream.adaptive.license_key=https://license.test/key
{H}/drm/1.mpd
#EXTGRP:Group From EXTGRP
#EXTINF:-1 tvg-id="grp.test",Group Via EXTGRP
{H}/grp/1.ts
#EXTINF:-1 tvg-id="hdr.test" group-title="Test Lab",Pipe Headers
{H}/hdr/1.ts|User-Agent=SyntheticAgent/2.0&Referer={H}/ref
#EXTINF:-1 tvg-id="cred.test" group-title="Test Lab",Credentials In URL
http://testuser:testpass@iptv.test/auth/1.ts
#EXTINF:-1 tvg-id="cred2.test" group-title="Test Lab",Credentials In Query
{H}/live/testuser/testpass/1.ts?username=testuser&password=testpass&token=testtoken123
''')
w("m3u/catchup.m3u", f'''#EXTM3U catchup="default" catchup-days="7" catchup-source="{H}/timeshift.php?username=testuser&password=testpass&stream=1&utc={{utc}}&duration={{duration:60}}" catchup-timezone="UTC"
#EXTINF:-1 tvg-id="cu1.test" catchup="default" catchup-days="7" catchup-source="{H}/ts/1?utc={{utc}}&utcend={{utcend}}" group-title="Catchup",Default Mode
{H}/live/cu1.ts
#EXTINF:-1 tvg-id="cu2.test" catchup="append" catchup-days="3" catchup-source="?offset=-{{duration}}&start={{start}}" group-title="Catchup",Append Mode
{H}/live/cu2.ts
#EXTINF:-1 tvg-id="cu3.test" catchup="shift" catchup-days="2" group-title="Catchup",Shift Mode
{H}/live/cu3.ts
#EXTINF:-1 tvg-id="cu4.test" catchup="xc" catchup-days="5" catchup-correction="-1" catchup-timezone="Europe/London" group-title="Catchup",XC Mode Corrected
{H}/live/testuser/testpass/4.ts
#EXTINF:-1 tvg-id="cu5.test" tv_archive="1" tv_archive_duration="3" group-title="Catchup",Dispatcharr tv_archive Attrs
{H}/live/cu5.ts
#EXTINF:-1 tvg-id="cu6.test" catchup="flussonic" catchup-days="1" group-title="Catchup",Unsupported Flussonic
{H}/live/cu6.ts
#EXTINF:-1 tvg-id="cu7.test" catchup="default" catchup-days="abc" catchup-source="" group-title="Catchup",Bad Days And Empty Source
{H}/live/cu7.ts
''')
w("m3u/malformed.m3u", '﻿' + (
  f'#EXTINF:-1 tvg-id="nohdr.test" group-title="Bad",Missing EXTM3U Header\r\n{H}/bad/1.ts\r\n'
  '#EXTM3U\r\n'
  f'#EXTINF:-1 tvg-id="crlf.test" group-title="Bad",CRLF Line Endings\r\n{H}/bad/2.ts\r\n'
  '#EXTINF:-1 tvg-id="nourl.test" group-title="Bad",EXTINF Without URL\r\n'
  f'#EXTINF:-1 tvg-id="after.test" group-title="Bad",After Missing URL\r\n{H}/bad/3.ts\r\n'
  f'{H}/bad/orphan-url.ts\r\n'
  f'#EXTINF:-1 tvg-id="unclosed.test group-title="Bad",Unclosed Quote\r\n{H}/bad/4.ts\r\n'
  f'#EXTINF:-1 tvg-id=noquotes.test group-title=Bad,Unquoted Attributes\r\n{H}/bad/5.ts\r\n'
  f'#EXTINF:-1 tvg-id="dup.test" tvg-id="dup2.test" group-title="Bad",Duplicate Attribute\r\n{H}/bad/6.ts\r\n'
  f'#EXTINF:-1 tvg-id="empty.test" group-title="Bad",\r\n{H}/bad/7.ts\r\n'
  f'#EXTINF:-1,Bare No Attrs\r\n{H}/bad/8.ts\r\n'
  f'#EXTINF:abc tvg-id="dur.test" group-title="Bad",Non Numeric Duration\r\n{H}/bad/9.ts\r\n'
  f'#EXTINF:-1 tvg-id="comma.test" tvg-name="Name, With, Commas" group-title="Bad, Group",Display, Name, With, Commas\r\n{H}/bad/10.ts\r\n'
  f'#EXTINF:-1 tvg-id="uni.test" group-title="Ünïcödé 日本語",Chaîne Tëst 📺\r\n{H}/bad/11.ts\r\n'
  f'#EXTINF:-1 tvg-chno="abc" tvg-id="chno.test" group-title="Bad",Non Numeric Chno\r\n{H}/bad/12.ts\r\n'
  f'#EXTINF:-1 tvg-chno="1.5" tvg-id="chno2.test" group-title="Bad",Float Chno\r\n{H}/bad/13.ts\r\n'
  '#EXTINF:-1 tvg-id="rel.test" group-title="Bad",Relative URL\r\n/relative/path.ts\r\n'
  '#EXTINF:-1 tvg-id="rtmp.test" group-title="Bad",RTMP Scheme\r\nrtmp://iptv.test/live/1\r\n'
  '#EXTINF:-1 tvg-id="udp.test" group-title="Bad",UDP Multicast\r\nudp://@239.0.0.1:1234\r\n'
  f'#EXTINF:-1\ttvg-id="tab.test"\tgroup-title="Bad",Tab Separated\r\n{H}/bad/14.ts\r\n'
  f'#EXTINF:-1 tvg-id="long.test" tvg-logo="{H}/{"x" * 4000}.png" group-title="Bad",Very Long Attribute\r\n{H}/bad/15.ts\r\n'
  '\r\n   \r\n# plain comment line\r\n#EXTINF:-1 tvg-id="last.test" group-title="Bad",No Trailing Newline\r\n' + f'{H}/bad/16.ts'))
w("m3u/README.md", """# M3U fixtures (synthetic)

- `basic.m3u`, `aux_lines.m3u` (EXTVLCOPT / KODIPROP / EXTGRP / pipe headers / credentials in URLs for redaction tests),
  `catchup.m3u` (default/append/shift/xc/flussonic/bad values, Dispatcharr `tv_archive` attrs), `malformed.m3u`
  (BOM, CRLF, orphan lines, unclosed quotes, duplicates, long attribute, odd schemes, no trailing newline).
- `gen-large-m3u.py` (in `tools/fixtures/`) writes a 20k-entry playlist to `m3u/generated/` (gitignored).
- Hosts use `.test`; credentials are synthetic (`testuser` / `testpass`).
""")

# ---------------- XMLTV ----------------
def xml(rel, body, enc="UTF-8", mode="w"):
    w(rel, f'<?xml version="1.0" encoding="{enc}"?>\n<tv generator-info-name="downlink-fixtures">\n{body}</tv>\n', enc=enc.lower() if enc != "UTF-8" else "utf-8")
ch = lambda i, n: f'<channel id="{i}"><display-name>{n}</display-name></channel>\n'
xml("xmltv/timezones.xml", ch("tz.test", "TZ Channel") +
    '<programme start="20260101120000 +0000" stop="20260101130000 +0000" channel="tz.test"><title>UTC Offset</title></programme>\n'
    '<programme start="20260101130000 +0100" stop="20260101140000 +0100" channel="tz.test"><title>Plus One Offset</title></programme>\n'
    '<programme start="20260101080000 -0500" stop="20260101090000 -0500" channel="tz.test"><title>Minus Five Offset</title></programme>\n'
    '<programme start="20260101150000" stop="20260101160000" channel="tz.test"><title>No Offset (assume UTC)</title></programme>\n'
    '<programme start="202601011600 +0000" stop="202601011700 +0000" channel="tz.test"><title>No Seconds</title></programme>\n'
    '<programme start="20260101170000+0000" stop="20260101180000+0000" channel="tz.test"><title>No Space Before Offset</title></programme>\n'
    '<programme start="20260101180000 +0530" stop="20260101190000 +0530" channel="tz.test"><title>Half Hour Offset</title></programme>\n')
xml("xmltv/edge_cases.xml",
    '<channel id="a.test"><display-name lang="en">Channel A</display-name><display-name>A Alias</display-name><display-name>101</display-name>'
    '<icon src="http://epg.test/a.png" width="100" height="100"/><url>http://epg.test/a</url></channel>\n' +
    ch("b.test", "Channel B &amp; Co") + ch("empty-name.test", "") +
    '<programme start="20260101120000 +0000" stop="20260101130000 +0000" channel="a.test">'
    '<title lang="en">Entities &amp; &lt;Tags&gt; &quot;Quotes&quot; &#169;</title><title lang="de">Titel auf Deutsch</title>'
    '<sub-title>Sub Title</sub-title><desc lang="en"><![CDATA[CDATA <b>description</b> & more]]></desc>'
    '<credits><director>Test Director</director><actor role="Lead">Test Actor</actor></credits>'
    '<date>2025</date><category lang="en">Drama</category><category lang="en">Test</category><icon src="http://epg.test/p.jpg"/>'
    '<episode-num system="xmltv_ns">0.4.0/1</episode-num><episode-num system="onscreen">S01E05</episode-num><episode-num system="dd_progid">EP000000010005</episode-num>'
    '<video><aspect>16:9</aspect><quality>HDTV</quality></video><audio><stereo>stereo</stereo></audio>'
    '<previously-shown start="20250101000000 +0000"/><premiere>Premiere</premiere><new/><live/><rating system="TEST"><value>PG</value></rating>'
    '<star-rating><value>3/5</value></star-rating></programme>\n'
    '<programme start="20260101130000 +0000" channel="a.test"><title>Missing Stop</title></programme>\n'
    '<programme start="20260101140000 +0000" stop="20260101140000 +0000" channel="a.test"><title>Zero Length</title></programme>\n'
    '<programme start="20260101160000 +0000" stop="20260101150000 +0000" channel="a.test"><title>Stop Before Start</title></programme>\n'
    '<programme start="20260101120000 +0000" stop="20260101140000 +0000" channel="b.test"><title>Overlap One</title></programme>\n'
    '<programme start="20260101130000 +0000" stop="20260101150000 +0000" channel="b.test"><title>Overlap Two</title></programme>\n'
    '<programme start="20260101120000 +0000" stop="20260101130000 +0000" channel="a.test"><title>Duplicate Of First</title></programme>\n'
    '<programme start="20260101120000 +0000" stop="20260101130000 +0000" channel="unknown.test"><title>Unknown Channel</title></programme>\n'
    '<programme start="20260101120000 +0000" stop="20260101130000 +0000" channel="empty-name.test"><title></title></programme>\n'
    '<programme start="20260101120000 +0000" stop="20260101130000 +0000" channel="b.test"></programme>\n'
    '<programme start="notadate" stop="alsonot" channel="b.test"><title>Unparseable Times</title></programme>\n'
    f'<programme start="20260101200000 +0000" stop="20260101210000 +0000" channel="b.test"><title>Long Description</title><desc>{"lorem ipsum " * 2000}</desc></programme>\n')
w("xmltv/latin1.xml", '<?xml version="1.0" encoding="ISO-8859-1"?>\n<tv>\n<channel id="l1.test"><display-name>Caf\xe9 Cha\xeene</display-name></channel>\n'
  '<programme start="20260101120000 +0000" stop="20260101130000 +0000" channel="l1.test"><title>Cr\xe8me Br\xfbl\xe9e</title></programme>\n</tv>\n', enc="latin-1")
w("xmltv/no_header.xml", '<tv><channel id="nh.test"><display-name>No XML Declaration</display-name></channel>'
  '<programme start="20260101120000 +0000" stop="20260101130000 +0000" channel="nh.test"><title>Bare</title></programme></tv>\n')
w("xmltv/truncated.xml", '<?xml version="1.0"?>\n<tv>\n<channel id="t.test"><display-name>Truncated</display-name></channel>\n'
  '<programme start="20260101120000 +0000" stop="20260101130000 +0000" channel="t.test"><title>Complete</title></programme>\n'
  '<programme start="20260101130000 +0000" stop="20260101140000 +0000" channel="t.test"><title>Cut off mid-ele')
w("xmltv/basic.xml.gz", gzip.compress(('<?xml version="1.0" encoding="UTF-8"?>\n<tv><channel id="gz.test"><display-name>Gzipped</display-name></channel>'
  '<programme start="20260101120000 +0000" stop="20260101130000 +0000" channel="gz.test"><title>From Gzip</title></programme></tv>\n').encode(), mtime=0), mode="wb")
print("ok")
