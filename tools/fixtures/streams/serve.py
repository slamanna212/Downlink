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
import argparse, os, subprocess, sys, time, threading
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
        if u.path.startswith("/ch/") and u.path.endswith(".ts"):
            c = BY_SLUG.get(u.path[4:-3])
            if c and os.path.isfile(os.path.join(MEDIA, c["clip"] + ".ts")): return self._stream(c)
        self._send(404, "not found", "text/plain")

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
