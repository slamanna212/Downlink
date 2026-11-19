#!/usr/bin/env python3
"""Write a synthetic playlist (+ matching XMLTV channel list) for load tests. Not committed: output is gitignored.
Usage: gen-large-m3u.py [count=20000] [out=tools/fixtures/m3u/generated/large.m3u]"""
import os, sys
n = int(sys.argv[1]) if len(sys.argv) > 1 else 20000
here = os.path.dirname(os.path.abspath(__file__))
out = sys.argv[2] if len(sys.argv) > 2 else os.path.join(here, "m3u", "generated", "large.m3u")
os.makedirs(os.path.dirname(out), exist_ok=True)
groups = [f"Test Group {g:03d}" for g in range(200)]
with open(out, "w", encoding="utf-8") as f:
    f.write('#EXTM3U x-tvg-url="http://iptv.test/epg.xml"\n')
    for i in range(1, n + 1):
        g = groups[i % len(groups)]
        catch = ' catchup="default" catchup-days="3" catchup-source="http://iptv.test/ts/%d?utc={utc}&duration={duration:60}"' % i if i % 10 == 0 else ""
        f.write(f'#EXTINF:-1 tvg-id="gen-{i}.test" tvg-name="Generated {i}" tvg-logo="http://iptv.test/logo/{i}.png" tvg-chno="{i}" group-title="{g}"{catch},Generated Channel {i}\n')
        f.write(f"http://iptv.test/live/{i}.ts\n")
print(f"wrote {n} entries to {out}")
