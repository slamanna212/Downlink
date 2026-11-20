#!/usr/bin/env python3
"""Write a synthetic XMLTV guide for the large-playlist load test (matches gen-large-m3u.py ids).
Not committed: output is gitignored. Synthetic titles only.
Usage: gen-large-xmltv.py [channels=4000] [days=7] [out=tools/fixtures/xmltv/generated/large.xml]
Programmes are hourly, from the start of yesterday to `days` days ahead of it."""
import datetime as dt, os, sys

channels = int(sys.argv[1]) if len(sys.argv) > 1 else 4000
days = int(sys.argv[2]) if len(sys.argv) > 2 else 7
here = os.path.dirname(os.path.abspath(__file__))
out = sys.argv[3] if len(sys.argv) > 3 else os.path.join(here, "xmltv", "generated", "large.xml")
os.makedirs(os.path.dirname(out), exist_ok=True)

start = (dt.datetime.now(dt.timezone.utc) - dt.timedelta(days=1)).replace(hour=0, minute=0, second=0, microsecond=0)
fmt = lambda t: t.strftime("%Y%m%d%H%M%S +0000")
words = ["Morning", "Evening", "Synthetic", "Test", "Pattern", "Signal", "Late", "Weekend", "Archive", "Showcase"]
n = 0
with open(out, "w", encoding="utf-8") as f:
    f.write('<?xml version="1.0" encoding="UTF-8"?>\n<tv generator-info-name="downlink-synthetic">\n')
    for c in range(1, channels + 1):
        f.write(f'<channel id="gen-{c}.test"><display-name>Generated Channel {c}</display-name></channel>\n')
    for c in range(1, channels + 1):
        for h in range(days * 24):
            t0 = start + dt.timedelta(hours=h)
            t1 = t0 + dt.timedelta(hours=1)
            title = f"{words[(c + h) % 10]} {words[(c * 7 + h) % 10]} {h % 97}"
            f.write(f'<programme start="{fmt(t0)}" stop="{fmt(t1)}" channel="gen-{c}.test">'
                    f'<title>{title}</title><desc>Synthetic description for programme {n} on channel {c}.</desc></programme>\n')
            n += 1
    f.write("</tv>\n")
print(f"wrote {n} programmes for {channels} channels to {out}")
