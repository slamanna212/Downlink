#!/usr/bin/env python3
"""Idempotent bootstrap of the local test Dispatcharr using its own REST API.

Creates (if missing): admin user, API key, M3U account (synthetic playlist from the host stream server),
XMLTV EPG source, channels from streams with EPG assigned, catch-up streams, one Redirect-profile channel.
Credentials live only in tools/dev/.env.local (gitignored). Never contacts a /proxy stream endpoint.

Env: DISPATCHARR_URL (default http://localhost:9191), STREAM_PORT (8088).
"""
import json, os, secrets, subprocess, sys, time, urllib.request, urllib.error

HERE = os.path.dirname(os.path.abspath(__file__))
ENV_FILE = os.path.join(HERE, ".env.local")
BASE = os.environ.get("DISPATCHARR_URL", "http://localhost:9191").rstrip("/")
STREAM_PORT = os.environ.get("STREAM_PORT", "8088")
ADMIN_USER = "downlink-admin"
CONTAINER = os.environ.get("DISPATCHARR_CONTAINER", "dispatcharr-test")
M3U_NAME, EPG_NAME = "Synthetic Playlist", "Synthetic EPG"

def log(*a): print("[bootstrap]", *a, flush=True)

def load_env():
    d = {}
    if os.path.exists(ENV_FILE):
        for line in open(ENV_FILE):
            if "=" in line and not line.startswith("#"):
                k, v = line.strip().split("=", 1); d[k] = v
    return d

def save_env(d):
    with open(ENV_FILE, "w") as f:
        f.write("# Local test Dispatcharr credentials. Synthetic, gitignored. Do not commit.\n")
        for k, v in d.items(): f.write(f"{k}={v}\n")
    os.chmod(ENV_FILE, 0o600)

class Api:
    def __init__(self): self.headers = {}
    def call(self, method, path, body=None, ok=(200, 201, 202, 204), raw=False):
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(BASE + path, data=data, method=method,
                                     headers={"Content-Type": "application/json", **self.headers})
        try:
            with urllib.request.urlopen(req, timeout=60) as r:
                txt = r.read().decode(); code = r.status
        except urllib.error.HTTPError as e:
            txt = e.read().decode(); code = e.code
        try: js = json.loads(txt) if txt else None
        except ValueError: js = txt
        if code not in ok: raise RuntimeError(f"{method} {path} -> {code}: {str(js)[:300]}")
        return js
    get = lambda self, p, **k: self.call("GET", p, **k)
    post = lambda self, p, b=None, **k: self.call("POST", p, b if b is not None else {}, **k)
    patch = lambda self, p, b, **k: self.call("PATCH", p, b, **k)
    def results(self, path):
        out, page = [], 1
        while True:
            sep = "&" if "?" in path else "?"
            r = self.get(f"{path}{sep}page={page}&page_size=500")
            if isinstance(r, list): return r
            out += r["results"]
            if not r.get("next"): return out
            page += 1

def bridge_ip():
    try:
        return subprocess.check_output(["docker", "network", "inspect", "bridge", "-f",
                                        "{{(index .IPAM.Config 0).Gateway}}"], text=True).strip()
    except Exception:
        return "172.17.0.1"

def main():
    env = load_env(); api = Api()
    st = api.get("/api/accounts/initialize-superuser/")
    if not st["superuser_exists"]:
        pw = secrets.token_urlsafe(18)
        api.post("/api/accounts/initialize-superuser/", {"username": ADMIN_USER, "password": pw})
        env.update(DISPATCHARR_USER=ADMIN_USER, DISPATCHARR_PASSWORD=pw); save_env(env)
        log("created admin user")
    if "DISPATCHARR_PASSWORD" not in env:
        sys.exit("admin exists but .env.local has no password; run dispatcharr-down.sh --purge and retry")
    tok = api.post("/api/accounts/token/", {"username": env["DISPATCHARR_USER"], "password": env["DISPATCHARR_PASSWORD"]})
    api.headers = {"Authorization": f"Bearer {tok['access']}"}

    me = api.get("/api/accounts/users/me/")
    if me.get("api_key"):
        key = me["api_key"]
    else:
        key = api.post("/api/accounts/api-keys/generate/")["key"]; log("generated API key")
    env["DISPATCHARR_URL"] = BASE; env["DISPATCHARR_API_KEY"] = key; save_env(env)

    gw = bridge_ip()
    host_for_container = f"host.docker.internal:{STREAM_PORT}"
    playlist = f"http://{host_for_container}/playlist.m3u?streamhost={host_for_container}&redirecthost={gw}:{STREAM_PORT}"
    epg_url = f"http://{host_for_container}/epg.xml"
    api.headers = {"X-API-Key": key}

    # --- M3U account
    accts = {a["name"]: a for a in api.results("/api/m3u/accounts/")}
    if M3U_NAME not in accts:
        a = api.post("/api/m3u/accounts/", {"name": M3U_NAME, "server_url": playlist, "account_type": "STD",
                                            "max_streams": 0, "is_active": True, "refresh_interval": 0,
                                            "auto_enable_new_groups_live": True})
        log("created M3U account", a["id"]); accts[M3U_NAME] = a
    else:
        api.patch(f"/api/m3u/accounts/{accts[M3U_NAME]['id']}/", {"server_url": playlist})
    acct = accts[M3U_NAME]

    # --- EPG source
    srcs = {s["name"]: s for s in api.results("/api/epg/sources/")}
    if EPG_NAME not in srcs:
        s = api.post("/api/epg/sources/", {"name": EPG_NAME, "source_type": "xmltv", "url": epg_url,
                                           "is_active": True, "refresh_interval": 0})
        log("created EPG source", s["id"]); srcs[EPG_NAME] = s
    else:
        api.patch(f"/api/epg/sources/{srcs[EPG_NAME]['id']}/", {"url": epg_url})
    epg = srcs[EPG_NAME]

    # --- wait for streams; refresh the account if needed
    def streams():
        return [s for s in api.results("/api/channels/streams/") if s.get("m3u_account") == acct["id"]]
    for attempt in range(60):
        ss = streams()
        if len(ss) >= 10: break
        if attempt in (5, 25): api.post(f"/api/m3u/refresh/{acct['id']}/")
        time.sleep(2)
    else:
        sys.exit(f"M3U streams did not appear (have {len(streams())})")
    log("streams:", len(ss))

    # --- channels
    existing = {c["name"]: c for c in api.results("/api/channels/channels/")}
    for s in sorted(ss, key=lambda x: x.get("stream_chno") or 0):
        if s["name"] in existing: continue
        body = {"stream_id": s["id"], "name": s["name"]}
        if s.get("stream_chno"): body["channel_number"] = s["stream_chno"]
        c = api.post("/api/channels/channels/from-stream/", body)
        existing[c["name"]] = c; log("channel", c["name"])

    # --- wait for EPG data then assign
    for attempt in range(60):
        eds = api.results(f"/api/epg/epgdata/?epg_source={epg['id']}")
        if len(eds) >= 10: break
        if attempt == 10: api.post("/api/epg/import/", {"id": epg["id"]}, ok=(200, 201, 202, 400))
        time.sleep(2)
    else:
        sys.exit("EPG data did not import")
    by_tvg = {e["tvg_id"]: e for e in eds}
    redirect = next(p for p in api.get("/api/core/streamprofiles/") if p["name"] == "Redirect")
    stream_by_name = {s["name"]: s for s in ss}
    for name, c in existing.items():
        tvg = stream_by_name.get(name, {}).get("tvg_id")
        patch = {}
        if tvg and c.get("tvg_id") != tvg: patch["tvg_id"] = tvg
        if tvg in by_tvg and c.get("epg_data_id") != by_tvg[tvg]["id"]: patch["epg_data_id"] = by_tvg[tvg]["id"]
        if name.startswith("Redirect") and c.get("stream_profile_id") != redirect["id"]:
            patch["stream_profile_id"] = redirect["id"]
        if patch: api.patch(f"/api/channels/channels/{c['id']}/", patch); log("patched", name, list(patch))

    final = api.results("/api/channels/channels/")
    log(f"done: {len(final)} channels; catch-up: {[c['name'] for c in final if c.get('is_catchup')]}")
    log(f"API key stored in {ENV_FILE}")

main()
