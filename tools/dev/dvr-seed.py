#!/usr/bin/env python3
"""Seed DVR state on the local test Dispatcharr via its REST API (synthetic channels only). Idempotent.

  - recurring rule "Synthetic Weekday Rule" (Mon/Wed/Fri 20:00-21:00 UTC on Channel One, 30 days from today)
  - series rule (tvg_id test-news.test, title "Pattern Playhouse", exact) + evaluate
  - one-off recording "Synthetic Completed Recording" on Test News if none exists (starts now, 45 s: completes in ~1 min)
  - with --upcoming: also a one-off starting in ~70 s lasting 2 min ("Synthetic Upcoming Recording")
Dispatcharr rejects recordings whose end is in the past ("End time must be in the future."), so a completed
recording can only be produced by recording a short live window and waiting.
Recording makes Dispatcharr itself pull the synthetic channel from the local stream server; the app is not involved.
"""
import datetime as dt, json, os, sys, urllib.request, urllib.error
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
HERE = os.path.dirname(os.path.abspath(__file__))
E = dict(l.strip().split("=", 1) for l in open(os.path.join(HERE, ".env.local")) if "=" in l and not l.startswith("#"))

def call(method, path, body=None, ok=(200, 201, 204)):
    req = urllib.request.Request(E["DISPATCHARR_URL"] + path, method=method, data=None if body is None else json.dumps(body).encode(),
                                 headers={"X-API-Key": E["DISPATCHARR_API_KEY"], "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=60) as r: code, txt = r.status, r.read().decode()
    except urllib.error.HTTPError as e: code, txt = e.code, e.read().decode()
    js = json.loads(txt) if txt else None
    if code not in ok: raise RuntimeError(f"{method} {path} -> {code} {txt[:200]}")
    return js

iso = lambda t: t.strftime("%Y-%m-%dT%H:%M:%SZ")

def main():
    now = dt.datetime.now(dt.timezone.utc)
    chans = {c["name"]: c for c in call("GET", "/api/channels/channels/?page=1&page_size=500")["results"]}
    recs = call("GET", "/api/channels/recordings/")
    title = lambda r: (r["custom_properties"].get("program") or {}).get("title")

    if "Synthetic Weekday Rule" not in [r["name"] for r in call("GET", "/api/channels/recurring-rules/")]:
        call("POST", "/api/channels/recurring-rules/", {"channel": chans["Channel One"]["id"], "days_of_week": [0, 2, 4],
             "start_time": "20:00:00", "end_time": "21:00:00", "enabled": True, "name": "Synthetic Weekday Rule",
             "start_date": now.date().isoformat(), "end_date": (now + dt.timedelta(days=30)).date().isoformat()})
        print("created recurring rule")
    if not any(r["title"] == "Pattern Playhouse" for r in call("GET", "/api/channels/series-rules/")["rules"]):
        call("POST", "/api/channels/series-rules/", {"tvg_id": "test-news.test", "title": "Pattern Playhouse", "mode": "all", "title_mode": "exact"})
        print("created series rule")
    call("POST", "/api/channels/series-rules/evaluate/", {})

    if not any(title(r) == "Synthetic Completed Recording" for r in recs):
        call("POST", "/api/channels/recordings/", {"channel": chans["Test News"]["id"], "start_time": iso(now - dt.timedelta(seconds=5)),
             "end_time": iso(now + dt.timedelta(seconds=45)), "custom_properties": {"program": {"title": "Synthetic Completed Recording", "description": "short"}}})
        print("scheduled short recording (completes in ~1 min)")
    if "--upcoming" in sys.argv:
        s = now + dt.timedelta(seconds=70)
        r = call("POST", "/api/channels/recordings/", {"channel": chans["Channel One"]["id"], "start_time": iso(s), "end_time": iso(s + dt.timedelta(minutes=2)),
                 "custom_properties": {"program": {"title": "Synthetic Upcoming Recording", "description": "one-off"}}})
        print("scheduled upcoming recording", r["id"], r["start_time"])

main()
