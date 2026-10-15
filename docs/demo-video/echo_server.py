"""A tiny server for the demo film: answers every request with what it received.

    python docs/demo-video/echo_server.py [port]      (default 18111)
"""
import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class Echo(BaseHTTPRequestHandler):
    def handle_any(self):
        n = int(self.headers.get("content-length") or 0)
        raw = self.rfile.read(n).decode("utf-8", "replace") if n else ""
        try:
            received = json.loads(raw) if raw else None
        except ValueError:
            received = raw
        body = json.dumps({
            "id": "ord_1042",
            "status": "received",
            "method": self.command,
            "path": self.path,
            "content_type": self.headers.get("content-type"),
            "received": received,
        }).encode()
        self.send_response(201 if self.command == "POST" else 200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    do_GET = do_POST = do_PUT = do_PATCH = do_DELETE = handle_any

    def log_message(self, *args):
        pass


ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1]) if len(sys.argv) > 1 else 18111), Echo).serve_forever()
