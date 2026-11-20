#!/usr/bin/env python3
"""Serve looping live-ish MPEG-TS channels, an M3U playlist and a dynamic XMLTV on one port.

  /ch/<slug>.ts     continuous TS; 'ffmpeg' mode = ffmpeg -re -stream_loop -1 -c copy (clean continuous timestamps),
                    'raw' mode = the clip byte-looped at real-time pace (timestamp discontinuities survive)
  /playlist.m3u     M3U; stream URLs use ?streamhost=host:port (default: the Host header)
  /epg.xml          XMLTV generated relative to now
  /health           200 ok
A channel with "drop_after" closes the connection mid-playback after that many seconds.
Every stream request is logged to stdout (tools/dev uses the log to confirm the app-side network discipline).
"""
import argparse, json, os, subprocess, sys, time, threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlparse, parse_qs
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import epg

HERE = os.path.dirname(os.path.abspath(__file__))
MEDIA = os.path.abspath(os.path.join(HERE, "..", "media"))
BY_SLUG = {c["slug"]: c for c in epg.CHANNELS}

class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    def log_message(self, fmt, *a):
        sys.stdout.write("%s %s\n" % (time.strftime("%H:%M:%S"), fmt % a)); sys.stdout.flush()

    def _send(self, code, body, ctype):
        b = body.encode() if isinstance(body, str) else body
        self.send_response(code); self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(b))); self.send_header("Connection", "close")
        self.end_headers()
        if self.command != "HEAD": self.wfile.write(b)

    do_HEAD = lambda self: self.do_GET()

    def do_GET(self):
        u = urlparse(self.path); q = parse_qs(u.query)
        host = self.headers.get("Host", "localhost")
        if u.path == "/health": return self._send(200, "ok", "text/plain")
        if u.path == "/epg.xml": return self._send(200, epg.xmltv(), "application/xml")
        if u.path == "/playlist.m3u":
            sh = q.get("streamhost", [host])[0]; rh = q.get("redirecthost", [None])[0]
            return self._send(200, epg.m3u(sh, f"http://{host}/epg.xml", rh), "audio/x-mpegurl")
        if u.path == "/player_api.php": return self._xc_api(q, host)
        parts = u.path.strip("/").split("/")
        # Xtream: /live/U/P/<id>.ts, /timeshift/U/P/<min>/<start>/<id>.ts, /streaming/timeshift.php?...
        xc = {c["xc_id"]: c for c in epg.xc_channels()}
        try:
            if parts[0] == "live" and len(parts) == 4 and self._xc_auth(parts[1], parts[2]):
                return self._stream_if(xc.get(int(parts[3].split(".")[0])))
            if parts[0] == "timeshift" and len(parts) == 6 and self._xc_auth(parts[1], parts[2]):
                return self._stream_if(xc.get(int(parts[5].split(".")[0])), minutes=int(parts[3]), start=parts[4])
            if u.path == "/streaming/timeshift.php" and self._xc_auth(q.get("username", [""])[0], q.get("password", [""])[0]):
                return self._stream_if(xc.get(int(q["stream"][0])), minutes=int(q.get("duration", ["60"])[0]), start=q.get("start", [""])[0])
        except (ValueError, KeyError):
            pass
        if parts[0] == "img" and len(parts) == 2 and parts[1].endswith(".png") and "/" not in parts[1][:-4]:
            return self._file(os.path.join(MEDIA, "img", parts[1]), "image/png")
        if parts[0] in ("movie", "series") and len(parts) == 4 and self._xc_auth(parts[1], parts[2]):
            ext = parts[3].rsplit(".", 1)[-1]
            if ext in ("mp4", "mkv"):
                return self._file(os.path.join(MEDIA, "vod_clip." + ext), "video/mp4" if ext == "mp4" else "video/x-matroska")
        if u.path.startswith("/ch/") and u.path.endswith(".ts"):
            c = BY_SLUG.get(u.path[4:-3])
            if c and os.path.isfile(os.path.join(MEDIA, c["clip"] + ".ts")): return self._stream(c)
        self._send(404, "not found", "text/plain")

    def _file(self, path, ctype):
        """Static file with single-range support (VOD seeking)."""
        if not os.path.isfile(path): return self._send(404, "not found", "text/plain")
        size = os.path.getsize(path); start, end, code = 0, size - 1, 200
        rng = self.headers.get("Range", "")
        if rng.startswith("bytes="):
            a, _, b = rng[6:].partition("-")
            try:
                start = int(a) if a else size - int(b); end = int(b) if (a and b) else size - 1; code = 206
            except ValueError: pass
            end = min(end, size - 1)
            if start > end: return self._send(416, "bad range", "text/plain")
        self.send_response(code); self.send_header("Content-Type", ctype); self.send_header("Accept-Ranges", "bytes")
        self.send_header("Content-Length", str(end - start + 1))
        if code == 206: self.send_header("Content-Range", f"bytes {start}-{end}/{size}")
        self.send_header("Connection", "close"); self.end_headers()
        if self.command == "HEAD": return
        try:
            with open(path, "rb") as f:
                f.seek(start); left = end - start + 1
                while left > 0:
                    d = f.read(min(65536, left))
                    if not d: break
                    self.wfile.write(d); left -= len(d)
        except (BrokenPipeError, ConnectionResetError): pass

    def _xc_auth(self, user, pw): return user == epg.XC_USER and pw == epg.XC_PASS

    def _stream_if(self, c, minutes=None, start=None):
        if not c: return self._send(404, "unknown stream", "text/plain")
        if start is not None:  # archive request: log it, and stop after the requested programme length
            sys.stdout.write("%s TIMESHIFT stream=%s start=%s minutes=%s\n" % (time.strftime("%H:%M:%S"), c["xc_id"], start, minutes))
            c = dict(c, drop_after=max(1, min(minutes, 120)) * 60)
        return self._stream(c)

    def _xc_api(self, q, host):
        a = q.get("action", [""])[0]
        if not self._xc_auth(q.get("username", [""])[0], q.get("password", [""])[0]):
            return self._send(200, json.dumps({"user_info": {"auth": 0}}), "application/json")
        body = {"": epg.xc_login(host), "get_live_categories": epg.xc_categories(),
                "get_live_streams": epg.xc_live_streams(host),
                "get_vod_categories": epg.VOD_CATEGORIES, "get_vod_streams": epg.xc_vod_streams(host),
                "get_series_categories": epg.SERIES_CATEGORIES, "get_series": epg.xc_series(host),
                "get_vod_info": epg.xc_vod_info(host, int(q.get("vod_id", ["0"])[0])),
                "get_series_info": epg.xc_series_info(host, int(q.get("series_id", ["0"])[0]))}.get(a, [])
        self._send(200, json.dumps(body), "application/json")

    def _stream(self, c):
        clip = os.path.join(MEDIA, c["clip"] + ".ts")
        self.send_response(200); self.send_header("Content-Type", "video/mp2t")
        self.send_header("Connection", "close"); self.end_headers()
        if self.command == "HEAD": return
        deadline = time.time() + c["drop_after"] if c.get("drop_after") else None
        try:
            if c["mode"] == "ffmpeg":
                p = subprocess.Popen(["ffmpeg", "-hide_banner", "-loglevel", "error", "-re", "-stream_loop", "-1",
                                      "-i", clip, "-c", "copy", "-f", "mpegts", "-"], stdout=subprocess.PIPE)
                try:
                    while True:
                        d = p.stdout.read(188 * 56)
                        if not d: break
                        self.wfile.write(d)
                        if deadline and time.time() > deadline: break
                finally:
                    p.kill(); p.wait()
            else:
                size = os.path.getsize(clip)
                dur = float(subprocess.check_output(["ffprobe", "-v", "error", "-show_entries", "format=duration",
                                                    "-of", "csv=p=0", clip]).decode().strip() or 20)
                rate = size / dur; chunk = 188 * 56; sent_t0 = time.time(); sent = 0
                while True:
                    with open(clip, "rb") as f:
                        while True:
                            d = f.read(chunk)
                            if not d: break
                            self.wfile.write(d); sent += len(d)
                            lag = sent / rate - (time.time() - sent_t0)
                            if lag > 0: time.sleep(lag)
                            if deadline and time.time() > deadline: return
        except (BrokenPipeError, ConnectionResetError):
            pass
        self.close_connection = True

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--bind", default="0.0.0.0"); ap.add_argument("--port", type=int, default=8088)
    a = ap.parse_args()
    s = ThreadingHTTPServer((a.bind, a.port), H); s.daemon_threads = True
    print(f"serving on {a.bind}:{a.port}, media={MEDIA}", flush=True)
    s.serve_forever()
main()
