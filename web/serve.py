#!/usr/bin/env python3
"""Dev server for the opsbox UI with caching turned off, so edits show up on a normal reload.
Usage: python3 serve.py [port]   (default 8099)"""
import http.server, os, sys

class NoCache(http.server.SimpleHTTPRequestHandler):
    def end_headers(self):
        self.send_header("Cache-Control", "no-store, must-revalidate")
        self.send_header("Expires", "0")
        super().end_headers()

os.chdir(os.path.dirname(os.path.abspath(__file__)))
port = int(sys.argv[1]) if len(sys.argv) > 1 else 8099
print(f"opsbox UI on http://localhost:{port}  (no cache)")
http.server.ThreadingHTTPServer(("127.0.0.1", port), NoCache).serve_forever()
